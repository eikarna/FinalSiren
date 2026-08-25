use uuid::Uuid;

#[derive(Clone)]
pub struct Config {
    pub uuid: Uuid,
    pub proxy_addr: String,
    pub proxy_port: u16,
}
