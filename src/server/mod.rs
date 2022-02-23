use futures::future;
use tokio::net::{TcpListener, UdpSocket};

use crate::{
    setup_logging, EyreResult, ServerArgs, ACK_MSG, EMPTY_MSG, HELLO_MSG, MAX_OPEN_FILES_COUNT,
    MSG_BUFFER_LENGTH,
};

#[derive(Debug, Copy, Clone)]
pub enum ServerResult {
    Success,
    PortAlreadyUsed,
    Unknown,
    Finished,
}

pub async fn handle_server(args: ServerArgs) -> EyreResult<ServerResult> {
    setup_logging(args.verbose)?;
    let (min, max) = args.port_range;

    let mut port_count = (max + 1) - min;
    let mut first_port = min;

    while port_count > 0 {
        let min = first_port;
        let max = first_port
            + if port_count > MAX_OPEN_FILES_COUNT {
                MAX_OPEN_FILES_COUNT
            } else {
                port_count
            };

        future::join_all((min..max).map(|port| spawn_tcp_udp_listener("0.0.0.0", port)))
            .await
            .into_iter()
            .try_for_each(|res| {
                let res = res?;
                let res_tcp = res.0.map(|_s| ());
                let res_udp = res.1.map(|_s| ());
                EyreResult::from_iter([res_tcp, res_udp])
            })?;

        first_port = max;
        port_count -= max - min;
    }

    Ok(ServerResult::Finished)
}

async fn spawn_tcp_listener(address: &str, port: u32) -> EyreResult<ServerResult> {
    let server_address = format!("{}:{}", address, port);

    match TcpListener::bind(&server_address).await {
        Ok(listener) => {
            log::info!("TCP Listener spawn on {}", server_address);

            match listener.accept().await {
                Ok((_stream, src)) => {
                    log::debug!("TCP Connection from {} to {}", src, server_address);
                    Ok(ServerResult::Success)
                }
                Err(e) => {
                    log::error!("TCP Error on {} => {}", server_address, e);
                    Ok(ServerResult::Unknown)
                }
            }
        }
        Err(_err) => {
            log::debug!("TCP address already used {}", server_address);
            Ok(ServerResult::PortAlreadyUsed)
        }
    }
}

async fn spawn_udp_listener(address: &str, port: u32) -> EyreResult<ServerResult> {
    let server_address = format!("{}:{}", address, port);

    match UdpSocket::bind(&server_address).await {
        Ok(listener) => {
            log::info!("UDP Listener spawn on {}", server_address);
            let mut buf = [0; MSG_BUFFER_LENGTH];

            loop {
                match listener.recv_from(&mut buf).await {
                    Ok((len, src)) => {
                        log::debug!("UDP Connection from {} to {}", src, server_address);

                        if &buf[..len] == HELLO_MSG {
                            listener.send_to(ACK_MSG, &src).await?;
                            return Ok(ServerResult::Success);
                        } else {
                            listener.send_to(EMPTY_MSG, &src).await?;
                        }
                    }
                    Err(e) => {
                        log::error!("UDP Error on {} => {}", server_address, e);
                        return Ok(ServerResult::Unknown);
                    }
                };
            }
        }
        Err(_err) => {
            log::debug!("UDP address already used {}", server_address);
            Ok(ServerResult::PortAlreadyUsed)
        }
    }
}

async fn spawn_tcp_udp_listener(
    address: &str,
    port: u32,
) -> EyreResult<(EyreResult<ServerResult>, EyreResult<ServerResult>)> {
    Ok(future::join(
        spawn_tcp_listener(address, port),
        spawn_udp_listener(address, port),
    )
    .await)
}
