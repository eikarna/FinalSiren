use super::ProxyStream;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use worker::*;

impl <'a> ProxyStream<'a> {
    pub async fn process_vless(&mut self) -> Result<()> {
        // ignore version
        self.read_u8().await?;
        
        // read uuid
        let mut user_id = [0u8; 16];
        self.read_exact(&mut user_id).await?;
        
        // read protobuf with a pre-allocated buffer to reduce allocations
        let m_len = self.read_u8().await?;
        let mut protobuf = [0u8; 64]; // small fixed-size buffer; larger payloads rare
        let len = (m_len as usize).min(protobuf.len());
        self.read_exact(&mut protobuf[..len]).await?;

        // read instruction
        let network_type = self.read_u8().await?;
        let is_tcp = network_type == 1;

        // read port and address
        let remote_port = {
            let mut port = [0u8; 2];
            self.read_exact(&mut port).await?;
            ((port[0] as u16) << 8) | (port[1] as u16)
        };
        let remote_addr = crate::common::parse_addr(self).await?;

        if is_tcp {
            let addr_pool = [
                (remote_addr.clone(), remote_port),
                (self.config.proxy_addr.clone(), self.config.proxy_port)
            ];

            // send header
            self.write(&[0u8; 2]).await?;
            for (target_addr, target_port) in addr_pool {
                if let Err(e) = self.handle_tcp_outbound(target_addr, target_port).await {
                    console_error!("error handling tcp: {}", e)
                }
            }
        } else {
            if let Err(e) = self.handle_udp_outbound().await {
                console_error!("error handling udp: {}", e)
            }
        }

        Ok(())
    }
}