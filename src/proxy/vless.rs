use super::ProxyStream;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use worker::*;

impl<'a> ProxyStream<'a> {
    pub async fn process_vless(&mut self) -> Result<()> {
        // Version (1 byte)
        let _version = self.read_u8().await?;

        // User UUID (16 bytes)
        let mut user_id = [0u8; 16];
        self.read_exact(&mut user_id).await?;

        // Addons / protobuf (1 byte len + N bytes)
        let m_len = self.read_u8().await?;
        if m_len > 0 {
            let mut protobuf = vec![0u8; m_len as usize];
            self.read_exact(&mut protobuf).await?;
        }

        // Command / Instruction (1: TCP, 2: UDP, 3: Mux)
        let network_type = self.read_u8().await?;
        let is_tcp = network_type == 1;

        // Port (2 bytes big endian)
        let remote_port = {
            let mut port = [0u8; 2];
            self.read_exact(&mut port).await?;
            ((port[0] as u16) << 8) | (port[1] as u16)
        };

        // Destination address
        let remote_addr = crate::common::parse_addr(self).await?;

        if is_tcp {
            // Write VLESS response header (version 0, addon length 0)
            self.write_all(&[0u8, 0u8]).await?;

            // Determine target: If proxy relay is configured, connect to relay;
            // Otherwise connect DIRECTLY to requested remote destination.
            let (target_addr, target_port) = if !self.config.proxy_addr.is_empty() {
                (self.config.proxy_addr.clone(), self.config.proxy_port)
            } else {
                (remote_addr, remote_port)
            };

            if let Err(e) = self.handle_tcp_outbound(target_addr, target_port).await {
                console_error!("error handling tcp outbound: {}", e);
            }
        } else {
            if let Err(e) = self.handle_udp_outbound().await {
                console_error!("error handling udp outbound: {}", e);
            }
        }

        Ok(())
    }
}
