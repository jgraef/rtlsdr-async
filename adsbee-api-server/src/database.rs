use std::ops::{
    Deref,
    DerefMut,
};

use serde::{
    Serialize,
    de::DeserializeOwned,
};
use sqlx::{
    PgConnection,
    PgPool,
    Postgres,
    types::Json,
};

#[derive(Debug, thiserror::Error)]
#[error("database error")]
pub enum Error {
    Sqlx(#[from] sqlx::error::Error),
    Migrate(#[from] sqlx::migrate::MigrateError),
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self, Error> {
        let pool = PgPool::connect(database_url).await?;

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    pub async fn transaction(&self) -> Result<Transaction<'_>, Error> {
        let inner = self.pool.begin().await?;
        Ok(Transaction { inner })
    }
}

#[derive(Debug)]
pub struct Transaction<'c> {
    inner: sqlx::Transaction<'c, Postgres>,
}

impl<'c> Deref for Transaction<'c> {
    type Target = PgConnection;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl<'c> DerefMut for Transaction<'c> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

impl<'c> Transaction<'c> {
    pub async fn commit(self) -> Result<(), Error> {
        self.inner.commit().await?;
        Ok(())
    }

    pub async fn rollback(self) -> Result<(), Error> {
        self.inner.rollback().await?;
        Ok(())
    }

    pub async fn get_metadata<T: DeserializeOwned>(
        &mut self,
        key: &str,
    ) -> Result<Option<T>, Error> {
        if let Some(row) = sqlx::query!("select value from metadata where key = $1", key)
            .fetch_optional(&mut *self.inner)
            .await?
        {
            Ok(Some(serde_json::from_value(row.value)?))
        }
        else {
            Ok(None)
        }
    }

    pub async fn set_metadata<T: Serialize>(&mut self, key: &str, value: &T) -> Result<(), Error> {
        sqlx::query_unchecked!("insert into metadata (key, value) values ($1, $2) on conflict (key) do update set value = $2", key, Json(value)).execute(&mut *self.inner).await?;
        Ok(())
    }
}
