use crate::nfs::{agents_fs, ReadOnlyAgentsFs};
use anyhow::{ensure, Result};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinSet,
};

async fn read(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>> {
    let mut message = Vec::new();
    loop {
        let header = stream.read_u32().await?;
        let count = (header & 0x7fff_ffff) as usize;
        ensure!(
            message.len() + count <= 8 * 1024 * 1024,
            "NFS RPC record exceeds limit"
        );
        let before = message.len();
        message.resize(before + count, 0);
        stream.read_exact(&mut message[before..]).await?;
        if header & 0x8000_0000 != 0 {
            return Ok(message);
        }
    }
}

async fn write(stream: &mut (impl AsyncWrite + Unpin), message: &[u8]) -> Result<()> {
    stream.write_u32(0x8000_0000 | message.len() as u32).await?;
    stream.write_all(message).await?;
    Ok(())
}

async fn requests(
    client: &mut (impl AsyncRead + Unpin),
    backend: &mut (impl AsyncWrite + Unpin),
    fs: &ReadOnlyAgentsFs,
    port: u16,
    replies: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    loop {
        let request = read(client).await?;
        match crate::nfs::acl::reply(fs, &request, port).await? {
            Some(response) => replies.send(response).await?,
            None => write(backend, &request).await?,
        }
    }
}

async fn responses(
    backend: &mut (impl AsyncRead + Unpin),
    replies: mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    loop {
        replies.send(read(backend).await?).await?;
    }
}

async fn deliver(
    client: &mut (impl AsyncWrite + Unpin),
    mut replies: mpsc::Receiver<Vec<u8>>,
) -> Result<()> {
    while let Some(response) = replies.recv().await {
        write(client, &response).await?;
    }
    Ok(())
}

async fn connection(
    client: TcpStream,
    backend: SocketAddr,
    fs: Arc<ReadOnlyAgentsFs>,
    port: u16,
) -> Result<()> {
    client.set_nodelay(true)?;
    let backend = TcpStream::connect(backend).await?;
    backend.set_nodelay(true)?;
    let (mut client_read, mut client_write) = client.into_split();
    let (mut backend_read, mut backend_write) = backend.into_split();
    let (send, receive) = mpsc::channel(16);
    // Retransmitted RPCs may have no backend response. Keep reading requests;
    // XIDs associate replies, while one bounded writer keeps frames intact.
    tokio::try_join!(
        requests(
            &mut client_read,
            &mut backend_write,
            &fs,
            port,
            send.clone()
        ),
        responses(&mut backend_read, send),
        deliver(&mut client_write, receive),
    )?;
    Ok(())
}

pub(super) async fn serve(listener: TcpListener, backend: SocketAddr, root: PathBuf) -> Result<()> {
    let fs = Arc::new(agents_fs(root));
    let port = listener.local_addr()?.port();
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            client = listener.accept() => {
                let (client, _) = client?;
                connections.spawn(connection(client, backend, fs.clone(), port));
            },
            result = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = result { tracing::warn!(%error, "NFS connection task failed"); }
            },
        }
    }
}
