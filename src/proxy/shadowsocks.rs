use super::ProxyStream;

use tokio::io::AsyncReadExt;
use worker::*;

impl<'a> ProxyStream<'a> {
    pub async fn process_shadowsocks(&mut self) -> Result<()> {
        let remote_addr = crate::common::parse_addr(self).await?;
        let remote_port = {
            let mut port = [0u8; 2];
            self.read_exact(&mut port).await?;
            ((port[0] as u16) << 8) | (port[1] as u16)
        };

        let (target_addr, target_port) = if !self.config.proxy_addr.is_empty() {
            (self.config.proxy_addr.clone(), self.config.proxy_port)
        } else {
            (remote_addr, remote_port)
        };

        if let Err(e) = self.handle_tcp_outbound(target_addr, target_port).await {
            console_error!("error handling shadowsocks tcp: {}", e);
        }

        Ok(())
    }
}
