use futures::future;
use tokio::net::{TcpListener, UdpSocket};

use crate::{
    setup_logging, EyreResult, ServerArgs, ACK_MSG, EMPTY_MSG, HELLO_MSG, MSG_BUFFER_LENGTH,
};

pub async fn handle_server(args: ServerArgs) -> EyreResult<()> {
    setup_logging(args.verbose)?;

    future::join_all(
        (args.port_range.0..=args.port_range.1).map(|port| spawn_tcp_udp_listener("0.0.0.0", port)),
    )
    .await
    .into_iter()
    .try_for_each(|res| {
        let res = res?;
        let res_tcp = res.0;
        let res_udp = res.1;
        EyreResult::from_iter([res_tcp, res_udp])
    })?;

    Ok(())
}

async fn spawn_tcp_listener(address: &str, port: u32) -> EyreResult<()> {
    log::info!("TCP Listener spawn on {}:{}", address, port);

    let listener = TcpListener::bind(&format!("{}:{}", address, port)).await?;

    loop {
        match listener.accept().await {
            Ok((_stream, src)) => {
                log::debug!("TCP Connection from {} to {}:{}", src, address, port);
            }
            Err(e) => {
                log::error!("TCP Error on {}:{} => {}", address, port, e);
            }
        }
    }
}

async fn spawn_udp_listener(address: &str, port: u32) -> EyreResult<()> {
    log::info!("UDP Listener spawn on {}:{}", address, port);

    let listener = UdpSocket::bind(&format!("{}:{}", address, port)).await?;
    let mut buf = [0; MSG_BUFFER_LENGTH];

    loop {
        match listener.recv_from(&mut buf).await {
            Ok((len, src)) => {
                log::debug!("UDP Connection from {} to {}:{}", src, address, port);

                if &buf[..len] == HELLO_MSG {
                    listener.send_to(ACK_MSG, &src).await?;
                } else {
                    listener.send_to(EMPTY_MSG, &src).await?;
                }
            }
            Err(e) => {
                log::error!("UDP Error on {}:{} => {}", address, port, e);
            }
        };
    }
}

async fn spawn_tcp_udp_listener(
    address: &str,
    port: u32,
) -> EyreResult<(EyreResult<()>, EyreResult<()>)> {
    Ok(future::join(
        spawn_tcp_listener(address, port),
        spawn_udp_listener(address, port),
    )
    .await)
}
