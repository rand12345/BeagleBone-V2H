use chrono::Duration;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteQueryResult;
use sqlx::{migrate::MigrateDatabase, Sqlite, SqlitePool};
use sqlx::{
    types::{
        chrono::{DateTime, Local, Utc},
        Json,
    },
    FromRow,
};
use sqlx_core::pool::PoolOptions;
use std::error::Error;
use tokio::time::{sleep, timeout, Instant};

use crate::error::IndraError;
use crate::POOL;

use super::meter::METER;
use super::mqtt::{MqttChademo, CHADEMO_DATA};

const DB_URL: &str = "sqlite://database.db";

pub async fn init(update_millisecs: u64) -> Result<(), IndraError> {
    let row_data = CHADEMO_DATA.clone();
    let update_period = std::time::Duration::from_millis(update_millisecs);
    loop {
        let instant = Instant::now();

        let row = *row_data.read().await;
        if row.state.is_inactive() {
            // only record activity
            sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }

        if let Some(db) = POOL.get() {
            match db.add_record(&(row).into()).await {
                Ok(sql) => log::info!("#{} db row added", sql.last_insert_rowid()),
                Err(e) => log::error!("db {e:?}"),
            };
        };
        let remaining = update_period - instant.elapsed();
        if remaining.as_millis().gt(&1) {
            sleep(remaining).await
        }
    }
}

#[derive(Clone, FromRow, Debug, Serialize, Deserialize)]
pub struct ChademoDbRow {
    pub id: u32,
    pub timestamp: chrono::DateTime<Utc>,
    pub dc_kw: f32,
    pub soc: u8,
    pub volts: u16,
    pub temp: f32,
    pub amps: f32,
    pub requested_amps: i16,
    pub fan: u8,
    pub meter_kw: f32,
}
impl From<MqttChademo> for ChademoDbRow {
    fn from(value: MqttChademo) -> Self {
        let meter_kw = match METER.clone().try_read().as_deref() {
            Ok(Some(val)) => val * 0.001,
            _ => 0.0,
        };
        Self {
            id: 0,
            timestamp: Utc::now(),
            dc_kw: value.volts * value.amps * 0.001,
            soc: value.soc as u8,
            volts: value.volts as u16,
            temp: value.temp,
            amps: value.amps,
            requested_amps: value.requested_amps as i16,
            fan: value.fan,
            meter_kw,
        }
    }
}
impl Default for ChademoDbRow {
    fn default() -> Self {
        Self {
            id: Default::default(),
            timestamp: Utc::now(),
            dc_kw: Default::default(),
            soc: Default::default(),
            volts: Default::default(),
            temp: Default::default(),
            amps: Default::default(),
            requested_amps: Default::default(),
            fan: Default::default(),
            meter_kw: Default::default(),
        }
    }
}

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new() -> Result<Self, IndraError> {
        let create_tables = if !Sqlite::database_exists(DB_URL).await.unwrap_or(false) {
            println!("Creating database {}", DB_URL);
            Sqlite::create_database(DB_URL)
                .await
                .map_err(|_e| IndraError::Error)?;
            true
        } else {
            println!("Database already exists");
            false
        };
        let pool = PoolOptions::new()
            .max_connections(5)
            .connect(DB_URL)
            .await
            .map_err(|_e| IndraError::Error)?;

        let db = Self { pool };
        if create_tables {
            let _ = db.create_table().await;
        }
        Ok(db)
    }
    pub async fn process_request(
        &self,
        request: Parameters,
    ) -> Result<Vec<ChademoDbRow>, Box<dyn Error>> {
        match request {
            Parameters::GetAllRecords => self.get_all_records().await,
            Parameters::GetLastNRecords(n) => self.get_last_n_records(n).await,
            Parameters::GetRecordsFromHours(h) => self.get_records_from_hours(h).await,
            Parameters::GetRecordsBetween((now, then)) => {
                self.get_records_between_hours(now, then).await
            }
        }
    }
    pub async fn add_record(
        &self,
        record: &ChademoDbRow,
    ) -> Result<SqliteQueryResult, Box<dyn Error>> {
        let mut conn = self.pool.acquire().await?;
        Ok(sqlx::query("INSERT INTO sensor_readings (timestamp, dc_kw, soc, volts, temp, amps, requested_amps, fan, meter_kw) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(Utc::now())
            .bind(record.dc_kw)
            .bind(record.soc)
            .bind(record.volts)
            .bind(record.temp)
            .bind(record.amps)
            .bind(record.requested_amps)
            .bind(record.fan)
            .bind(record.meter_kw)
            .execute(&mut *conn)
            .await
            ?)
    }
    pub async fn get_all_records(&self) -> Result<Vec<ChademoDbRow>, Box<dyn Error>> {
        Ok(
            sqlx::query_as::<_, ChademoDbRow>("SELECT * FROM sensor_readings")
                .fetch_all(&self.pool)
                .await?,
        )
    }
    pub async fn get_last_n_records(&self, n: usize) -> Result<Vec<ChademoDbRow>, Box<dyn Error>> {
        let query = format!(
            "SELECT * FROM sensor_readings ORDER BY timestamp DESC LIMIT {}",
            n
        );

        Ok(sqlx::query_as::<_, ChademoDbRow>(&query)
            .fetch_all(&self.pool)
            .await?)
    }
    async fn get_records_from_hours(
        &self,
        hours: impl Into<i64>,
    ) -> Result<Vec<ChademoDbRow>, Box<dyn Error>> {
        let now = Utc::now();
        let hours_ago = now - Duration::seconds(hours.into() * 3600);
        Ok(self.get_records_between_hours(hours_ago, now).await?)
        // let hours_ago = now - Duration::seconds(hours * 3600);
    }
    async fn get_records_between_hours(
        &self,
        then: impl Into<DateTime<Utc>>,
        now: impl Into<DateTime<Utc>>,
    ) -> Result<Vec<ChademoDbRow>, Box<dyn Error>> {
        Ok(sqlx::query_as::<_, ChademoDbRow>(
            "SELECT * FROM sensor_readings WHERE timestamp BETWEEN ? AND ?",
        )
        .bind(then.into())
        .bind(now.into())
        .fetch_all(&self.pool)
        .await?)
    }
    pub async fn create_table(&self) -> Result<(), Box<dyn Error>> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sensor_readings (
                id INTEGER PRIMARY KEY,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                dc_kw REAL,
                soc INTEGER,
                volts INTEGER,
                temp REAL,
                amps REAL,
                requested_amps INTEGER,
                fan INTEGER,
                meter_kw REAL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum Parameters {
    GetAllRecords,
    GetLastNRecords(usize),
    GetRecordsFromHours(i64),
    GetRecordsBetween((DateTime<Utc>, DateTime<Utc>)),
}

#[cfg(test)]
mod test {
    use super::*;
    #[tokio::test]
    async fn test_connect_db() {
        let db = Database::new().await;
        assert!(db.is_ok());
        let result = db.unwrap().create_table().await;
        assert!(result.is_ok());
    }
    #[tokio::test]
    async fn test_add_record() {
        let db = Database::new().await;
        assert!(db.is_ok());
        let db = db.unwrap();
        let result = db.add_record(&ChademoDbRow::default()).await;
        assert!(result.is_ok());
        println!("Query result: {:?}", result);

        let results = db.get_all_records().await;
        for result in results.unwrap() {
            println!("[{:?}] ", result);
        }
    }
}
