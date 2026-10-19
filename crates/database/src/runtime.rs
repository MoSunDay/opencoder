use anyhow::{Context, Result};
use std::{future::Future, sync::OnceLock};
use tokio::{runtime::Runtime, sync::Semaphore};

struct Executor {
    runtime: Runtime,
    admission: Semaphore,
}

fn executor(write: bool) -> &'static Executor {
    static READS: OnceLock<Executor> = OnceLock::new();
    static WRITES: OnceLock<Executor> = OnceLock::new();
    let (slot, threads, name) = if write {
        (&WRITES, 2, "database-write")
    } else {
        (&READS, 4, "database-read")
    };
    slot.get_or_init(|| Executor {
        runtime: tokio::runtime::Builder::new_multi_thread()
            .worker_threads(threads)
            .thread_name(name)
            .enable_all()
            .build()
            .expect("create database executor"),
        admission: Semaphore::new(128),
    })
}

pub(crate) async fn run<F, T>(write: bool, work: F) -> Result<T>
where
    F: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    let executor = executor(write);
    // Transfer ownership before waiting for executor capacity, including when
    // another database occupies the executor. Cancellation only detaches the
    // JoinHandle; an already admitted database operation still finishes.
    executor
        .runtime
        .spawn(async move {
            let _admission = executor
                .admission
                .acquire()
                .await
                .context("database executor closed")?;
            work.await
        })
        .await
        .context("database operation panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancellation_while_executor_is_full_keeps_owned_work() {
        let held = executor(true).admission.acquire_many(128).await.unwrap();
        let (sent, received) = tokio::sync::oneshot::channel();
        let mut operation = Box::pin(run(true, async move {
            sent.send(()).unwrap();
            Ok(())
        }));
        std::future::poll_fn(|context| {
            assert!(operation.as_mut().poll(context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(operation);
        drop(held);
        tokio::time::timeout(std::time::Duration::from_secs(5), received)
            .await
            .unwrap()
            .unwrap();
    }
}
