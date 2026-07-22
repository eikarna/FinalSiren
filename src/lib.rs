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

static PROXYIP_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^.+-\d+$").unwrap());
static PROXYKV_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^([A-Z]{2})").unwrap());

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
            kv.put("proxy_kv", &body)?
                .expiration_ttl(60 * 60 * 24)
                .execute()
                .await?;
            body
        }
    };

    Ok(serde_json::from_str(&proxy_kv_str)?)
}

// Base URL for GitHub raw content
static GITHUB_BASE_URL: &str = "https://raw.githubusercontent.com/eikarna/SirenWeb/refs/heads/main";

#[event(fetch)]
async fn main(req: Request, env: Env, _: Context) -> Result<Response> {
    let uuid = env
        .var("UUID")
        .map(|x| Uuid::parse_str(&x.to_string()).unwrap_or_default())?;
    let host = req.url()?.host().map(|x| x.to_string()).unwrap_or_default();
    let main_page_url = env.var("MAIN_PAGE_URL").map(|x| x.to_string()).unwrap();
    let sub_page_url = env.var("SUB_PAGE_URL").map(|x| x.to_string()).unwrap();
    let link_page_url = env.var("LINK_PAGE_URL").map(|x| x.to_string()).unwrap();
    let converter_page_url = env.var("CONVERTER_PAGE_URL").map(|x| x.to_string()).unwrap();

    let config = Config {
        uuid,
        proxy_addr: host,
        proxy_port: 443,
        main_page_url,
        sub_page_url,
        link_page_url,
        converter_page_url,

    };


    let url = req.url()?;
    let path = url.path();

    if path.starts_with("/css/") {
        return handle_css_file(req).await;
    } else if path.starts_with("/js/") {
        return handle_js_file(req).await;
    } else if path.starts_with("/images/") {
        return handle_image_file(req).await;
    }

    Router::with_data(config)
        .on_async("/check", check)
        .on_async("/", fe)
        .on_async("/sub", sub)
        .on_async("/link", link)
        .on_async("/converter", converter)
        .on_async("/:proxyip", tunnel)
        .run(req, env)
        .await
}

async fn handle_css_file(req: Request) -> Result<Response> {
    let url = req.url()?;
    let filename = url.path().strip_prefix("/css/").unwrap_or("");
    let css_url = format!("{}/css/{}", GITHUB_BASE_URL, filename);

    let fetch_req = Request::new(&css_url, Method::Get)?;
    let mut res = Fetch::Request(fetch_req).send().await?;

    if res.status_code() == 200 {
        let css = res.text().await?;
        let headers = Headers::new();
        headers.set("Content-Type", "text/css")?;
        Ok(Response::ok(css)?.with_headers(headers))
    } else {
        Response::error("CSS file not found", 404)
    }
}

async fn handle_js_file(req: Request) -> Result<Response> {
    let url = req.url()?;
    let filename = url.path().strip_prefix("/js/").unwrap_or("");
    let js_url = format!("{}/js/{}", GITHUB_BASE_URL, filename);

    let fetch_req = Request::new(&js_url, Method::Get)?;
    let mut res = Fetch::Request(fetch_req).send().await?;

    if res.status_code() == 200 {
        let js = res.text().await?;
        let headers = Headers::new();
        headers.set("Content-Type", "application/javascript")?;
        Ok(Response::ok(js)?.with_headers(headers))
    } else {
        Response::error("JavaScript file not found", 404)
    }
}

async fn handle_image_file(req: Request) -> Result<Response> {
    let url = req.url()?;
    let filename = url.path().strip_prefix("/images/").unwrap_or("");
    let image_url = format!("{}/images/{}", GITHUB_BASE_URL, filename);

    let mut req_init = RequestInit::new();
    let cf_props = CfProperties {
        cache_ttl: Some(86400), // Cache for 24 hours
        ..Default::default()
    };
    req_init.with_cf_properties(cf_props);
    let fetch_req = Request::new_with_init(&image_url, &req_init)?;
    let mut res = Fetch::Request(fetch_req).send().await?;

    if res.status_code() == 200 {
        let image_data = res.bytes().await?;
        let headers = Headers::new();

        if filename.ends_with(".png") {
            headers.set("Content-Type", "image/png")?;
        } else if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
            headers.set("Content-Type", "image/jpeg")?;
        } else if filename.ends_with(".svg") {
            headers.set("Content-Type", "image/svg+xml")?;
        } else if filename.ends_with(".gif") {
            headers.set("Content-Type", "image/gif")?;
        } else {
            headers.set("Content-Type", "application/octet-stream")?;
        }

        headers.set("Cache-Control", "public, max-age=86400")?;
        Ok(Response::from_bytes(image_data)?.with_headers(headers))
    } else {
        Response::error("Image file not found", 404)
    }
}

async fn get_response_from_url(url: String) -> Result<Response> {
    let req = Request::new(url.as_str(), Method::Get)?;

    let mut res = Fetch::Request(req).send().await?;
    Response::from_html(res.text().await?)
}

async fn fe(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    get_response_from_url(cx.data.main_page_url.clone()).await
}

async fn sub(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    get_response_from_url(cx.data.sub_page_url.clone()).await
}

async fn link(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    get_response_from_url(cx.data.link_page_url.clone()).await
}

async fn converter(_: Request, cx: RouteContext<Config>) -> Result<Response> {
    get_response_from_url(cx.data.converter_page_url.clone()).await
}

/// `GET /check` — probe proxy relays for liveness and return a JSON report.
///
/// Query parameters:
/// - `cc`    : restrict the sweep to a single country code (e.g. `ID`). When
///             omitted, relays are drawn from all countries.
/// - `limit` : max number of relays to probe in this request (default 20, hard
///             cap 50). Guards against exceeding Workers CPU/wall budgets.
/// - `timeout`: per-relay connect deadline in ms (default 3000).
///
/// Returns `{ "checked", "alive", "results": [{ addr, port, alive, latency_ms }] }`.
async fn check(req: Request, cx: RouteContext<Config>) -> Result<Response> {
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
        .unwrap_or(3000)
        .max(500);

    let kv = cx.kv("SIREN")?;
    let proxy_kv = load_proxy_kv(&kv).await?;

    // Collect candidate relay entries from the requested countries.
    let entries: Vec<String> = match &cc {
        Some(code) => proxy_kv
            .get(code)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(limit)
            .collect(),
        None => {
            // Round-robin across countries so the sweep samples breadth-first
            // instead of draining one country entirely before the next.
            let columns: Vec<&Vec<String>> = proxy_kv.values().collect();
            let mut picked = Vec::new();
            let mut i = 0;
            while picked.len() < limit {
                let mut progressed = false;
                for col in &columns {
                    if i < col.len() {
                        picked.push(col[i].clone());
                        progressed = true;
                        if picked.len() >= limit {
                            break;
                        }
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

    let targets: Vec<Target> = entries
        .iter()
        .filter_map(|raw| parse_target(raw))
        .collect();

    if targets.is_empty() {
        return Response::from_json(&serde_json::json!({
            "checked": 0,
            "alive": 0,
            "results": Vec::<ProbeResult>::new(),
            "note": match &cc {
                Some(code) => format!("no relays found for country code '{}'", code),
                None => "no relays available".to_string(),
            }
        }));
    }

    // Bounded concurrency: 8 in-flight connects keeps us within Workers'
    // outbound-socket guidance while still parallelizing the sweep.
    let results = probe_all(targets, 8, timeout_ms).await;
    let alive = results.iter().filter(|r| r.alive).count();

    Response::from_json(&serde_json::json!({
        "checked": results.len(),
        "alive": alive,
        "results": results,
    }))
}

async fn tunnel(req: Request, mut cx: RouteContext<Config>) -> Result<Response> {
    let mut proxyip = match cx.param("proxyip") {
        Some(p) => p.to_string(),
        None => return Response::error("Missing proxyip parameter", 400),
    };
    if PROXYKV_PATTERN.is_match(&proxyip) {
        let kvid_list: Vec<String> = proxyip.split(",").map(|s| s.to_string()).collect();
        let kv = cx.kv("SIREN")?;
        let proxy_kv = load_proxy_kv(&kv).await?;
        let mut rand_buf = [0u8, 1];
        getrandom::getrandom(&mut rand_buf).expect("failed generating random number");

        let kv_index = (rand_buf[0] as usize) % kvid_list.len();
        proxyip = kvid_list[kv_index].clone();

        let proxyip_index = (rand_buf[0] as usize) % proxy_kv[&proxyip].len();
        proxyip = proxy_kv[&proxyip][proxyip_index].clone().replace(":", "-");
    }

    if PROXYIP_PATTERN.is_match(&proxyip) {
        if let Some((addr, port_str)) = proxyip.split_once('-') {
            if let Ok(port) = port_str.parse() {
                cx.data.proxy_addr = addr.to_string();
                cx.data.proxy_port = port;
            }
        }
    }

    let upgrade = req.headers().get("Upgrade")?.unwrap_or("".to_string());
    if upgrade == "websocket" {
        let WebSocketPair { server, client } = WebSocketPair::new()?;
        server.accept()?;

        wasm_bindgen_futures::spawn_local(async move {
            let events = server.events().unwrap();
            if let Err(e) = ProxyStream::new(cx.data, &server, events).process().await {
                console_log!("[tunnel]: {}", e);
            }
        });

        Response::from_websocket(client)
    } else {
        Response::from_html("hi from wasm!")
    }
}
