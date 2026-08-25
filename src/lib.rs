mod common;
mod config;
mod proxy;

use crate::config::Config;
use crate::proxy::*;

use std::collections::HashMap;
use uuid::Uuid;
use worker::*;
use once_cell::sync::Lazy;
use regex::Regex;

static PROXYKV_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^([A-Z]{2}|ALL)").unwrap());

/// URL of the upstream `proxy_kv` relay list, cached in the `SIREN` KV namespace.
static PROXY_KV_URL: &str =
    "https://raw.githubusercontent.com/FoolVPN-ID/Nautica/refs/heads/main/kvProxyList.json";

/// Load the `proxy_kv` map (`{ "CC": ["ip:port", ...] }`) from the `SIREN` KV
/// namespace, populating it from GitHub on a cache miss. Shared by the tunnel
/// handler and the `/check` liveness endpoint so they always see the same list.
async fn load_proxy_kv(kv: &worker::kv::KvStore) -> Result<HashMap<String, Vec<String>>> {
    let cached = kv.get("proxy_kv").text().await?;
    let proxy_kv_str = match cached {
        Some(s) if !s.is_empty() => s,
        _ => {
            console_log!("getting proxy kv from github...");
            let req = Fetch::Url(Url::parse(PROXY_KV_URL)?);
            let mut res = req.send().await?;
            if res.status_code() != 200 {
                return Err(Error::from(format!(
                    "error getting proxy kv: {}",
                    res.status_code()
                )));
            }
            let body = res.text().await?.to_string();
            let _ = kv.put("proxy_kv", &body)?
                .expiration_ttl(60 * 60 * 24)
                .execute()
                .await;
            body
        }
    };

    Ok(serde_json::from_str(&proxy_kv_str)?)
}

#[event(fetch)]
async fn main(req: Request, env: Env, _: Context) -> Result<Response> {
    let uuid = env
        .var("UUID")
        .map(|x| Uuid::parse_str(&x.to_string()).unwrap_or_default())
        .unwrap_or_default();

    // Default: Empty proxy_addr means pure direct connect unless an explicit relay path is requested
    let config = Config {
        uuid,
        proxy_addr: String::new(),
        proxy_port: 0,
    };

    // Any WebSocket upgrade -> Handle Tunneling Directly
    let upgrade = req.headers().get("Upgrade")?.unwrap_or_default();
    if upgrade.eq_ignore_ascii_case("websocket") {
        return handle_ws_tunnel(req, env, config).await;
    }

    let url = req.url()?;
    let path = url.path();

    // Health & Check endpoints
    if path == "/check" {
        return check(req, env).await;
    } else if path == "/check/single" {
        return check_single(req).await;
    }

    // Pure Tunnel Node: Return lightweight JSON status for standard HTTP probes
    let mut headers = Headers::new();
    headers.set("Content-Type", "application/json; charset=utf-8")?;
    headers.set("Server", "Edge-Tunnel-Core")?;

    Ok(Response::ok(r#"{"status":"online","service":"tunnel-core","ws_active":true}"#)?
        .with_headers(headers))
}

async fn check(req: Request, env: Env) -> Result<Response> {
    let query: HashMap<String, String> = req
        .url()?
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let cc = query.get("cc").map(|s| s.to_uppercase());
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(20)
        .min(50)
        .max(1);
    let timeout_ms = query
        .get("timeout")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(2000)
        .max(500);

    let kv_res = env.kv("SIREN");
    let proxy_kv = match kv_res {
        Ok(kv) => load_proxy_kv(&kv).await.unwrap_or_default(),
        Err(_) => HashMap::new(),
    };

    let entries: Vec<String> = match &cc {
        Some(code) if code != "ALL" => proxy_kv
            .get(code)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(limit)
            .collect(),
        _ => {
            let columns: Vec<&Vec<String>> = proxy_kv.values().collect();
            let mut picked = Vec::new();
            let mut i = 0;
            while picked.len() < limit {
                let mut progressed = false;
                for col in &columns {
                    if i < col.len() {
                        picked.push(col[i].clone());
                        if picked.len() == limit {
                            progressed = true;
                            break;
                        }
                        progressed = true;
                    }
                }
                if !progressed {
                    break;
                }
                i += 1;
            }
            picked
        }
    };

    if entries.is_empty() {
        return Response::error("No proxy IPs found for this query", 404);
    }

    let mut checks = Vec::new();
    for entry in entries {
        let (ip, port) = match entry.split_once(':') {
            Some((ip, p)) => (ip.to_string(), p.parse::<u16>().unwrap_or(443)),
            None => (entry.clone(), 443),
        };

        checks.push(async move {
            let start = Date::now().as_millis();
            let socket = Socket::builder().connect(&ip, port);
            match socket {
                Ok(mut sock) => {
                    let opened = sock.opened().await;
                    let latency = Date::now().as_millis() - start;
                    let _ = sock.close();
                    if opened.is_ok() && (latency as u32) <= timeout_ms {
                        Some((format!("{}:{}", ip, port), latency))
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        });
    }

    let mut active = Vec::new();
    for check_fut in checks {
        if let Some(res) = check_fut.await {
            active.push(serde_json::json!({
                "proxy": res.0,
                "latency_ms": res.1,
                "status": "ACTIVE"
            }));
        }
    }

    let mut headers = Headers::new();
    headers.set("Content-Type", "application/json")?;
    Ok(Response::ok(serde_json::to_string(&active)?)?.with_headers(headers))
}

async fn check_single(req: Request) -> Result<Response> {
    let query: HashMap<String, String> = req
        .url()?
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let proxy = match query.get("proxy") {
        Some(p) => p,
        None => return Response::error("Missing 'proxy' parameter", 400),
    };

    let (ip, port) = match proxy.split_once(':') {
        Some((ip, p)) => (ip.to_string(), p.parse::<u16>().unwrap_or(443)),
        None => (proxy.clone(), 443),
    };

    let start = Date::now().as_millis();
    match Socket::builder().connect(&ip, port) {
        Ok(mut sock) => match sock.opened().await {
            Ok(_) => {
                let latency = Date::now().as_millis() - start;
                let _ = sock.close();
                let mut headers = Headers::new();
                headers.set("Content-Type", "application/json")?;
                Ok(Response::ok(serde_json::json!({
                    "proxy": format!("{}:{}", ip, port),
                    "latency_ms": latency,
                    "status": "ACTIVE"
                }).to_string())?.with_headers(headers))
            }
            Err(e) => Response::error(format!("Connection failed: {e}"), 502),
        },
        Err(e) => Response::error(format!("Socket creation failed: {e}"), 500),
    }
}

async fn handle_ws_tunnel(req: Request, env: Env, mut config: Config) -> Result<Response> {
    let url = req.url()?;
    let path = url.path().trim_start_matches('/').to_string();

    // If path is specified and not direct/root, parse custom relay ip-port
    if !path.is_empty() && path != "direct" && path != "vless" {
        let mut proxyip = path;
        let kv_res = env.kv("SIREN");
        if let Ok(kv) = kv_res {
            if let Ok(proxy_kv) = load_proxy_kv(&kv).await {
                let upper = proxyip.to_uppercase();
                if upper == "ALL" {
                    let all_ips: Vec<String> = proxy_kv.values().flatten().cloned().collect();
                    if !all_ips.is_empty() {
                        let mut rand_buf = [0u8; 2];
                        let _ = getrandom::getrandom(&mut rand_buf);
                        let idx = ((rand_buf[0] as usize) << 8 | (rand_buf[1] as usize)) % all_ips.len();
                        proxyip = all_ips[idx].replace(':', "-");
                    }
                } else if PROXYKV_PATTERN.is_match(&proxyip) {
                    let kvid_list: Vec<String> = upper.split(',').map(|s| s.trim().to_string()).collect();
                    let mut rand_buf = [0u8; 2];
                    let _ = getrandom::getrandom(&mut rand_buf);
                    let kv_index = (rand_buf[0] as usize) % kvid_list.len();
                    let selected_cc = &kvid_list[kv_index];

                    if let Some(list) = proxy_kv.get(selected_cc) {
                        if !list.is_empty() {
                            let proxyip_index = (rand_buf[1] as usize) % list.len();
                            proxyip = list[proxyip_index].clone().replace(':', "-");
                        }
                    }
                }
            }
        }

        // Parse ip-port or ip:port
        if let Some((addr, port_str)) = proxyip.split_once('-').or_else(|| proxyip.split_once(':')) {
            if let Ok(port) = port_str.parse::<u16>() {
                config.proxy_addr = addr.to_string();
                config.proxy_port = port;
            }
        }
    }

    let WebSocketPair { server, client } = WebSocketPair::new()?;
    server.accept()?;

    wasm_bindgen_futures::spawn_local(async move {
        let events = match server.events() {
            Ok(ev) => ev,
            Err(e) => {
                console_log!("[tunnel]: failed to get ws events: {}", e);
                return;
            }
        };
        if let Err(e) = ProxyStream::new(config, &server, events).process().await {
            console_log!("[tunnel]: {}", e);
        }
    });

    Response::from_websocket(client)
}
