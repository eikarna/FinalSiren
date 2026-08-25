use super::ProxyStream;

use tokio::io::AsyncReadExt;
use worker::*;

impl<'a> ProxyStream<'a> {
    pub async fn process_trojan(&mut self) -> Result<()> {
        // User ID / Hash (56 hex bytes)
        let mut _user_id = [0u8; 56];
        self.read_exact(&mut _user_id).await?;

        // CRLF (2 bytes)
        self.read_u16().await?;

        // Command (1: TCP, 3: UDP)
        let network_type = self.read_u8().await?;
        let is_tcp = network_type == 1;

        // Destination address & port
        let remote_addr = crate::common::parse_addr(self).await?;
        let remote_port = {
            let mut port = [0u8; 2];
            self.read_exact(&mut port).await?;
            ((port[0] as u16) << 8) | (port[1] as u16)
        };

        // CRLF (2 bytes)
        self.read_u16().await?;

        if is_tcp {
            let (target_addr, target_port) = if !self.config.proxy_addr.is_empty() {
                (self.config.proxy_addr.clone(), self.config.proxy_port)
            } else {
                (remote_addr, remote_port)
            };

            if let Err(e) = self.handle_tcp_outbound(target_addr, target_port).await {
                console_error!("error handling tcp trojan: {}", e);
            }
        } else {
            if let Err(e) = self.handle_udp_outbound().await {
                console_error!("error handling udp trojan: {}", e);
            }
        }

        Ok(())
    }
}
