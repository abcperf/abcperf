use crate::stats::{
    AlgoStats, ClientStats, HostInfo, ReplicaSample, ReplicaStats, RunInfo, Timestamp,
};
use anyhow::{anyhow, Ok, Result};
use clap::Args;
use futures::future;
use reqwest::{Body, Client as WebClient, StatusCode, Url};
use serde::{Deserialize, Serialize};
use shared_ids::{ClientId, IdIter, ReplicaId};
use std::{iter, mem};
use tracing::{debug, info, trace};
use uuid::Uuid;

#[derive(Args, Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SaveOpt {
    /// save collected stats on this clickhouse server
    #[clap(flatten)]
    clickhouse: ClickhouseConfig,
}

const LIMIT: usize = 500_000;

#[derive(Args, Clone, Debug, Deserialize, Serialize)]
#[group(requires_all = ["clickhouse_host", "clickhouse_user", "clickhouse_password", "clickhouse_database"])]
struct ClickhouseConfig {
    #[clap(long)]
    clickhouse_host: Option<String>,
    #[clap(long)]
    clickhouse_user: Option<String>,
    #[clap(long)]
    clickhouse_password: Option<String>,
    #[clap(long)]
    clickhouse_database: Option<String>,
}

trait TableName: Serialize {
    const ABCPERF_TABLE_NAME: &'static str;
}

#[derive(Debug, Serialize)]
struct RunsRow {
    run_id: Uuid,
    orchestrator_info: Uuid,

    #[serde(flatten)]
    run_info: RunInfo,
}

impl TableName for RunsRow {
    const ABCPERF_TABLE_NAME: &'static str = "runs";
}

#[derive(Debug, Serialize)]
struct ReplicasRow {
    run_id: Uuid,
    replica_id: ReplicaId,
    replica_info: Uuid,
    ticks_per_second: u64,
}

impl TableName for ReplicasRow {
    const ABCPERF_TABLE_NAME: &'static str = "replicas";
}

#[derive(Debug, Serialize)]
struct RoundsRow {
    run_id: Uuid,
    replica_id: ReplicaId,
    round_id: u64,
    timestamp: Timestamp,
}

impl TableName for RoundsRow {
    const ABCPERF_TABLE_NAME: &'static str = "rounds";
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct ReplicaSamplesRow {
    run_id: Uuid,
    replica_id: ReplicaId,
    sample_id: u64,
    #[serde(flatten)]
    sample: ReplicaSample,
}

impl TableName for ReplicaSamplesRow {
    const ABCPERF_TABLE_NAME: &'static str = "replica_samples";
}

#[derive(Debug, Serialize)]
struct HostInfoRow {
    id: Uuid,

    #[serde(flatten)]
    info: HostInfo,
}

impl TableName for HostInfoRow {
    const ABCPERF_TABLE_NAME: &'static str = "host_info";
}

#[derive(Debug, Serialize)]
struct ClientSamplesRow {
    run_id: Uuid,
    client_id: u64,
    sample_id: u64,
    timestamp: u64,
    processing_time: u64,
}

impl TableName for ClientSamplesRow {
    const ABCPERF_TABLE_NAME: &'static str = "client_samples";
}

impl SaveOpt {
    #[cfg(test)]
    pub(crate) fn discard() -> Self {
        Self {
            clickhouse: ClickhouseConfig {
                clickhouse_host: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
        }
    }

    pub(super) async fn save_orchestrator_info(
        &self,
        run_info: RunInfo,
        client_host_info: HostInfo,
    ) -> Result<Option<Uuid>> {
        Ok(
            if let ClickhouseConfig {
                clickhouse_database: Some(database),
                clickhouse_password: Some(password),
                clickhouse_host: Some(host),
                clickhouse_user: Some(user),
            } = &self.clickhouse
            {
                let url = Url::parse(&format!(
                    "https://{}:{}@{}/?database={}",
                    user, password, host, database
                ))?;

                let run_id = Uuid::new_v4();
                info!("saving to clickhouse with run_id {}", run_id);

                let web_client = WebClient::new();
                let orchestrator_info =
                    Self::clickhouse_host_info(&url, &web_client, client_host_info).await?;

                Self::clickhouse_insert_web_client(
                    &url,
                    &web_client,
                    iter::once(RunsRow {
                        run_id,
                        orchestrator_info,
                        run_info,
                    }),
                )
                .await?;

                Some(run_id)
            } else {
                info!("Skip storing to clickhouse");
                None
            },
        )
    }

    pub(crate) async fn save_client_samples(
        &self,
        run_id: Uuid,
        clients: Vec<ClientStats>,
    ) -> Result<()> {
        if let ClickhouseConfig {
            clickhouse_database: Some(database),
            clickhouse_password: Some(password),
            clickhouse_host: Some(host),
            clickhouse_user: Some(user),
        } = &self.clickhouse
        {
            info!("saving client samples");

            let url = Url::parse(&format!(
                "https://{}:{}@{}/?database={}",
                user, password, host, database
            ))?;
            let web_client = WebClient::new();

            let total_count: usize = clients.iter().map(|v| v.samples.len()).sum();
            let client_samples =
                IdIter::<ClientId>::default()
                    .zip(clients)
                    .flat_map(|(client_id, stats)| {
                        let ClientStats { samples } = stats;
                        samples
                            .into_iter()
                            .enumerate()
                            .map(move |(sample_id, sample)| ClientSamplesRow {
                                run_id,
                                client_id: client_id.as_u64(),
                                sample_id: sample_id as u64,
                                processing_time: sample.processing_time.into(),
                                timestamp: sample.timestamp.into(),
                            })
                    });

            let chunk_size = total_count / 4;

            info!(
                "saving {} client samples in 4 chunks of {}",
                total_count, chunk_size
            );

            let mut chunks = Vec::with_capacity(4);
            let mut chunk = Vec::with_capacity(chunk_size);
            for sample in client_samples {
                chunk.push(sample);
                if chunk.len() >= chunk_size {
                    chunks.push(chunk);
                    chunk = Vec::with_capacity(chunk_size);
                }
            }
            if !chunk.is_empty() {
                chunks.push(chunk);
            }

            let insert_tasks: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    let web_client = web_client.clone();
                    let url = url.clone();
                    tokio::spawn(async move {
                        Self::clickhouse_insert_web_client(&url, &web_client, chunk.into_iter())
                            .await
                            .unwrap()
                    })
                })
                .collect();
            future::join_all(insert_tasks).await;
        }
        Ok(())
    }

    pub(crate) async fn save_replica_samples(
        &self,
        run_id: Uuid,
        replica_id: ReplicaId,
        replica: ReplicaStats,
        algo: AlgoStats,
    ) -> Result<()> {
        let ClickhouseConfig {
            clickhouse_database: Some(database),
            clickhouse_password: Some(password),
            clickhouse_host: Some(host),
            clickhouse_user: Some(user),
        } = &self.clickhouse
        else {
            return Ok(());
        };
        info!("saving replica samples");
        let url = Url::parse(&format!(
            "https://{}:{}@{}/?database={}",
            user, password, host, database
        ))?;
        let web_client = WebClient::new();
        let ReplicaStats {
            samples,
            ticks_per_second,
            host_info,
        } = replica;
        let replica_info = Self::clickhouse_host_info(&url, &web_client, host_info).await?;
        Self::clickhouse_insert_web_client(
            &url,
            &web_client,
            iter::once(ReplicasRow {
                run_id,
                replica_id,
                replica_info,
                ticks_per_second,
            }),
        )
        .await?;
        Self::clickhouse_insert_web_client(
            &url,
            &web_client,
            samples
                .into_iter()
                .enumerate()
                .map(move |(sample_id, sample)| ReplicaSamplesRow {
                    run_id,
                    replica_id,
                    sample_id: sample_id as u64,
                    sample,
                }),
        )
        .await?;
        Self::clickhouse_insert_web_client(
            &url,
            &web_client,
            algo.round_transitions
                .into_iter()
                .enumerate()
                .map(move |(round_id, timestamp)| RoundsRow {
                    run_id,
                    replica_id,
                    round_id: round_id as u64,
                    timestamp,
                }),
        )
        .await?;
        Ok(())
    }

    async fn clickhouse_host_info(
        url: &Url,
        web_client: &WebClient,
        info: HostInfo,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        Self::clickhouse_insert_web_client(url, web_client, iter::once(HostInfoRow { id, info }))
            .await?;
        Ok(id)
    }

    async fn clickhouse_insert_web_client<T: TableName>(
        url: &Url,
        web_client: &WebClient,
        rows: impl Iterator<Item = T> + Sync + Send + 'static,
    ) -> Result<()> {
        debug!("inserting into table {}", T::ABCPERF_TABLE_NAME);
        let mut url = url.clone();
        url.query_pairs_mut().append_pair(
            "query",
            &format!("INSERT INTO {} FORMAT JSONEachRow", T::ABCPERF_TABLE_NAME),
        );

        let mut batches = 0usize;
        let mut entries = 0usize;
        let mut count = 0usize;
        let mut rows_string = Vec::with_capacity(LIMIT * 100);
        for row in rows {
            serde_json::to_writer(&mut rows_string, &row)?;

            count += 1;

            if count >= LIMIT {
                trace!("Inserting batch");
                Self::insert(web_client, url.clone(), mem::take(&mut rows_string)).await?;
                rows_string.reserve(LIMIT * 100);
                entries += count;
                batches += 1;
                count = 0;
            } else {
                rows_string.push(b'\n');
            }
        }

        if count > 0 {
            entries += count;
            batches += 1;
            trace!("Inserting batch");
            Self::insert(web_client, url.clone(), rows_string).await?;
        }
        debug!("Inserted {} entries in {} batches", entries, batches);
        Ok(())
    }

    async fn insert(web_client: &WebClient, url: Url, rows: Vec<u8>) -> Result<()> {
        let response = web_client.post(url).body(Body::from(rows)).send().await?;

        let status_code = response.status();
        if status_code != StatusCode::OK {
            let body = response.text().await?;
            return Err(anyhow!(
                "wrong status code it was {:?} {}",
                status_code,
                body
            ));
        }
        Ok(())
    }
}
