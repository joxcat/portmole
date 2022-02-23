use std::{
    net::{IpAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};

use eyre::{Context, ContextCompat};
use futures::{future, Future};
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
    let mut last_port_to_check = Some(min);
    let currently_checking_count = Arc::new(AtomicU32::new(0));
    let mut ports_futures: Vec<Box<dyn Future<Output = EyreResult<()>>>> =
        Vec::with_capacity((max + 1 - min) as usize);

    while last_port_to_check.is_some() {
        if currently_checking_count.load(Ordering::Relaxed) < MAX_OPEN_FILES_COUNT {
            let last_port = last_port_to_check.unwrap();
            let nb_ports_to_check =
                MAX_OPEN_FILES_COUNT - currently_checking_count.load(Ordering::Relaxed);

            (last_port..(last_port + nb_ports_to_check)).for_each(|port| {
                ports_futures.push(Box::new(handle_port(
                    address,
                    port,
                    Duration::from_millis(args.timeout),
                    currently_checking_count.clone(),
                )));
            });

            currently_checking_count.fetch_add(1, Ordering::Relaxed);

            if last_port == max {
                last_port_to_check = None;
            } else if last_port + nb_ports_to_check > max {
                last_port_to_check = Some(max);
            } else {
                last_port_to_check = Some(last_port + nb_ports_to_check);
            }
        } else {
            // Sleep some time to let the server spawn the listeners
            time::sleep(Duration::from_millis(2000)).await;
        }
    }

    Ok(ClientResult::Finished)
}

async fn handle_port(
    address: IpAddr,
    port: u32,
    timeout: Duration,
    currently_checking_count: Arc<AtomicU32>,
) -> EyreResult<()> {
    let _res = spawn_tcp_udp_connection(address, port, timeout).await;
    currently_checking_count.fetch_sub(1, Ordering::Relaxed);

    Ok(())
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
