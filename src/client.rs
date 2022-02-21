use std::{
    net::{IpAddr, ToSocketAddrs},
    time::Duration,
};

use eyre::{Context, ContextCompat};
use futures::future;
use tokio::{net::UdpSocket, time};
use trust_dns_resolver::{
    config::{ResolverConfig, ResolverOpts},
    AsyncResolver,
};

use crate::{
    setup_logging, ClientArgs, EyreResult, ACK_MSG, HELLO_MSG, MAX_OPEN_FILES_COUNT,
    MSG_BUFFER_LENGTH,
};

#[derive(Debug, Copy, Clone)]
pub enum ClientResult {
    Success,
    SuccessButBadACK,
    CannotConnect,
    Timeout,
    Unknown,
    Finished,
}

pub async fn handle_client(args: ClientArgs) -> EyreResult<ClientResult> {
    setup_logging(args.verbose)?;

    // Resolve domain to an IP
    let resolver = AsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default())?;
    let resp = resolver
        .lookup_ip(args.server.clone())
        .await
        .wrap_err("Cannot find this domain name")?;
    let address = resp
        .iter()
        .next()
        .wrap_err("Cannot resolve IP from this domain name")?;

    log::debug!("Server IP resolved from {} to {}", args.server, address);

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

        future::join_all((min..max).map(|port| {
            spawn_tcp_udp_connection(address, port, Duration::from_millis(args.timeout))
        }))
        .await
        .into_iter()
        .try_for_each(|res| {
            let res = res?;
            let res_tcp = res.0.map(|_c| ());
            let res_udp = res.1.map(|_c| ());
            EyreResult::from_iter([res_tcp, res_udp])
        })?;

        first_port = max;
        port_count -= max - min;

        // Sleep some time to let the server spawn the listeners
        time::sleep(Duration::from_millis(500)).await;
    }

    Ok(ClientResult::Finished)
}

async fn spawn_tcp_udp_connection(
    address: IpAddr,
    port: u32,
    timeout: Duration,
) -> EyreResult<(EyreResult<ClientResult>, EyreResult<ClientResult>)> {
    Ok(future::join(
        spawn_tcp_connection(address, port, timeout),
        spawn_udp_connection(address, port, timeout),
    )
    .await)
}

async fn spawn_tcp_connection(
    address: IpAddr,
    port: u32,
    timeout: Duration,
) -> EyreResult<ClientResult> {
    let sever_address = format!("{}:{}", address, port);

    match std::net::TcpStream::connect_timeout(
        &(address, port as u16)
            .to_socket_addrs()?
            .next()
            .wrap_err("Cannot convert address + port to socket address")?,
        timeout,
    ) {
        Ok(stream) => {
            log::info!("TCP Connection succeed to {}", stream.peer_addr()?);
            Ok(ClientResult::Success)
        }
        Err(_elapsed) => {
            log::debug!(
                "TCP Cannot connect to {}, timeout after {}ms",
                sever_address,
                timeout.as_millis()
            );
            Ok(ClientResult::Timeout)
        }
    }
}

async fn spawn_udp_connection(
    address: IpAddr,
    port: u32,
    timeout: Duration,
) -> EyreResult<ClientResult> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    let server_address = format!("{}:{}", address, port);

    match socket.connect(&server_address).await {
        Ok(()) => {
            let mut buf = [0; MSG_BUFFER_LENGTH];

            socket.send(HELLO_MSG).await?;
            match time::timeout(timeout, socket.recv(&mut buf)).await {
                Ok(recv_result) => {
                    let len = match recv_result {
                        Ok(recv_result) => recv_result,
                        Err(err) => {
                            log::trace!(
                                "UDP Cannot receive from {} because {}",
                                server_address,
                                err
                            );
                            log::debug!("UDP Cannot receive from {}", server_address);
                            return Ok(ClientResult::CannotConnect);
                        }
                    };

                    if &buf[..len] == ACK_MSG {
                        log::info!("UDP Connection succeed to {}", server_address);
                        Ok(ClientResult::Success)
                    } else {
                        log::debug!("UDP Connection successful but ACK message is not good");
                        Ok(ClientResult::SuccessButBadACK)
                    }
                }
                Err(_elapsed) => {
                    log::debug!(
                        "UDP Cannot receive from {}, timeout after {}ms",
                        server_address,
                        timeout.as_millis()
                    );
                    Ok(ClientResult::Timeout)
                }
            }
        }
        Err(err) => {
            log::trace!("UDP Cannot connect to {} because {}", server_address, err);
            log::debug!("UDP Cannot connect to {}", server_address);
            Ok(ClientResult::Unknown)
        }
    }
}
