use crate::nfs::{agents_fs, ReadOnlyAgentsFs};
use anyhow::{ensure, Result};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

async fn read(stream: &mut TcpStream) -> Result<Vec<u8>> {
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

async fn write(stream: &mut TcpStream, message: &[u8]) -> Result<()> {
    stream.write_u32(0x8000_0000 | message.len() as u32).await?;
    stream.write_all(message).await?;
    Ok(())
}

async fn connection(
    mut client: TcpStream,
    backend: SocketAddr,
    fs: Arc<ReadOnlyAgentsFs>,
    port: u16,
) -> Result<()> {
    client.set_nodelay(true)?;
    let mut backend = TcpStream::connect(backend).await?;
    backend.set_nodelay(true)?;
    loop {
        let request = read(&mut client).await?;
        let response = match crate::nfs::acl::reply(&fs, &request, port).await? {
            Some(response) => response,
            None => {
                write(&mut backend, &request).await?;
                read(&mut backend).await?
            }
        };
        write(&mut client, &response).await?;
    }
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
