//! Connection ownership shared with its originating pool.

use std::{fmt, ops::Deref, sync::Arc};
use turso::Connection;

use crate::pool::Inner;

/// A connection that keeps its originating pool alive.
///
/// Every clone owns another pool reference, so a writer's shared maintenance
/// lease is released only after the final escaped connection clone is gone. Fields drop in
/// declaration order: the native connection closes before the pool reference.
#[derive(Clone)]
pub struct ConnectionLease {
    connection: Connection,
    #[allow(dead_code)]
    pool: Arc<Inner>,
}

impl ConnectionLease {
    pub(crate) fn new(pool: Arc<Inner>, connection: Connection) -> Self {
        Self { connection, pool }
    }
}

impl Deref for ConnectionLease {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl fmt::Debug for ConnectionLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("ConnectionLease").finish()
    }
}
