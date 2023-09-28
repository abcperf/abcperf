use std::{fs::File, iter, path::PathBuf};

use crate::stats::{
    ClientSample, ClientStats, CpuInfo, HostInfo, KernelInfo, MessageCount, MessageCounts,
    ReplicaSample, ReplicaSampleStats, ReplicaStats, RxTx, Stats,
};
use anyhow::{anyhow, Ok, Result};
use clap::Args;
use futures::stream;
use reqwest::{Body, Client, StatusCode, Url};
use serde_json::{json, Value};
use tracing::{debug, info};
use uuid::Uuid;

#[derive(Args)]
pub(crate) struct SaveOpt {
    /// save collected stats as json
    #[clap(long, value_name = "path")]
    json: Vec<PathBuf>,

    /// save collected stats as pretty json
    #[clap(long, value_name = "path")]
    pretty_json: Vec<PathBuf>,

    /// save collected stats as yaml
    #[clap(long, value_name = "path")]
    yaml: Vec<PathBuf>,

    /// save collected stats in the bincode format
    #[clap(long, value_name = "path")]
    bincode: Vec<PathBuf>,

    /// save collected stats on this clickhouse server
    #[clap(long, value_name = "url")]
    clickhouse: Option<Url>,
}

impl SaveOpt {
    #[cfg(test)]
    pub(crate) fn discard() -> Self {
        Self {
            json: vec![],
            pretty_json: vec![],
            yaml: vec![],
            bincode: vec![],
            clickhouse: None,
        }
    }

    pub(super) async fn save_results(&self, results: Stats) -> Result<()> {
        Self::save_to_file(self.json.as_slice(), serde_json::to_writer, &results)?;
        Self::save_to_file(
            self.pretty_json.as_slice(),
            serde_json::to_writer_pretty,
            &results,
        )?;
        Self::save_to_file(self.yaml.as_slice(), serde_yaml::to_writer, &results)?;
        Self::save_to_file(self.bincode.as_slice(), bincode::serialize_into, &results)?;
        if let Some(url) = &self.clickhouse {
            Self::save_to_clickhouse(url, results).await?;
        }
        Ok(())
    }

    async fn save_to_clickhouse(url: &Url, results: Stats) -> Result<()> {
        info!("saving to clickhouse");
        let Stats {
            clients,
            replicas,
            client_host_info,
            abcperf_version,
            algo_version,
            start_time,
            end_time,
            algo_name,
            algo_config,
            client_config,
            n,
            t,
            experiment_duration,
            sample_delay,
            seed,
            network_latency_type,
            network_latency_config,
            network_drop_chance,
            network_duplicate_chance,
            description,
            f,
            omission_chance,
        } = results;

        let web_client = Client::new();

        let run_id = Uuid::new_v4();
        info!("assigned run_id {}", run_id);

        let client_info = Self::clickhouse_host_info(url, &web_client, &client_host_info).await?;

        Self::clickhouse_insert(
            url,
            &web_client,
            "runs",
            iter::once(json!({
                "run_id": run_id,
                "abcperf_version": abcperf_version,
                "algo_version": algo_version,
                "client_info": client_info,
                "start_time": start_time,
                "end_time": end_time,
                "algo_name": algo_name,
                "algo_config": algo_config,
                "client_config": client_config,
                "n": n,
                "t": t,
                "f": f,
                "omission_chance": omission_chance,
                "experiment_duration": experiment_duration,
                "replicas_sample_delay": sample_delay,
                "seed": seed,
                "replicas_network_latency_type": network_latency_type,
                "replicas_network_latency_config": network_latency_config,
                "replicas_network_drop_chance": network_drop_chance,
                "replicas_network_duplicate_chance": network_duplicate_chance,
                "description": description,
            })),
        )
        .await?;

        for (client_id, client) in clients.into_iter().enumerate() {
            let ClientStats { samples } = client;
            Self::clickhouse_insert(
                url,
                &web_client,
                "client_samples",
                samples
                    .into_iter()
                    .enumerate()
                    .map(move |(sample_id, sample)| {
                        let ClientSample {
                            timestamp,
                            processing_time,
                            replica_id,
                        } = sample;
                        json!({
                            "run_id": run_id,
                            "client_id": client_id,
                            "sample_id": sample_id,
                            "timestamp": timestamp,
                            "processing_time": processing_time,
                            "replica_id": replica_id,
                        })
                    }),
            )
            .await?;
        }

        for (replica_id, replica) in replicas.into_iter().enumerate() {
            let ReplicaStats {
                samples,
                ticks_per_second,
                host_info,
            } = replica;

            let replica_info = Self::clickhouse_host_info(url, &web_client, &host_info).await?;

            Self::clickhouse_insert(
                url,
                &web_client,
                "replicas",
                iter::once(json!({
                    "run_id": run_id,
                    "replica_id": replica_id,
                    "replica_info": replica_info,
                    "ticks_per_second": ticks_per_second,
                })),
            )
            .await?;

            Self::clickhouse_insert(
                url,
                &web_client,
                "replica_samples",
                samples
                    .into_iter()
                    .enumerate()
                    .map(move |(sample_id, sample)| {
                        let ReplicaSample {
                            timestamp,
                            stats:
                                ReplicaSampleStats {
                                    memory_usage_in_bytes,
                                    memory_used_by_stats,
                                    user_mode_ticks,
                                    kernel_mode_ticks,
                                    rx_bytes,
                                    tx_bytes,
                                    rx_datagrams,
                                    tx_datagrams,
                                    messages:
                                        MessageCounts {
                                            algo:
                                                MessageCount {
                                                    unicast:
                                                        RxTx {
                                                            rx: rx_algo_unicast_messages,
                                                            tx: tx_algo_unicast_messages,
                                                        },
                                                    broadcast:
                                                        RxTx {
                                                            rx: rx_algo_broadcast_messages,
                                                            tx: tx_algo_broadcast_messages,
                                                        },
                                                },
                                            server:
                                                MessageCount {
                                                    unicast:
                                                        RxTx {
                                                            rx: rx_server_unicast_messages,
                                                            tx: tx_server_unicast_messages,
                                                        },
                                                    broadcast:
                                                        RxTx {
                                                            rx: rx_server_broadcast_messages,
                                                            tx: tx_server_broadcast_messages,
                                                        },
                                                },
                                        },
                                },
                        } = sample;
                        json!({
                            "run_id": run_id,
                            "replica_id": replica_id,
                            "sample_id": sample_id,
                            "timestamp": timestamp,
                            "memory_usage_in_bytes": memory_usage_in_bytes,
                            "memory_used_by_stats": memory_used_by_stats,
                            "user_mode_ticks": user_mode_ticks,
                            "kernel_mode_ticks": kernel_mode_ticks,
                            "rx_bytes": rx_bytes,
                            "tx_bytes": tx_bytes,
                            "rx_datagrams": rx_datagrams,
                            "tx_datagrams": tx_datagrams,
                            "rx_algo_unicast_messages": rx_algo_unicast_messages,
                            "tx_algo_unicast_messages": tx_algo_unicast_messages,
                            "rx_algo_broadcast_messages": rx_algo_broadcast_messages,
                            "tx_algo_broadcast_messages": tx_algo_broadcast_messages,
                            "rx_server_unicast_messages": rx_server_unicast_messages,
                            "tx_server_unicast_messages": tx_server_unicast_messages,
                            "rx_server_broadcast_messages": rx_server_broadcast_messages,
                            "tx_server_broadcast_messages": tx_server_broadcast_messages,
                        })
                    }),
            )
            .await?;
        }

        info!("done saving to clickhouse");

        Ok(())
    }

    async fn clickhouse_host_info(
        url: &Url,
        web_client: &Client,
        host_info: &HostInfo,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();

        let HostInfo {
            hostname,
            user,
            memory,
            swap,
            cpu,
            kernel,
        } = host_info;

        let CpuInfo {
            common: cpu_common,
            cores: cpu_cores,
        } = cpu;

        let KernelInfo {
            version: kernel_version,
            flags: kernel_flags,
            extra: kernel_extra,
        } = kernel;

        Self::clickhouse_insert(
            url,
            web_client,
            "host_info",
            iter::once(json!({
                "id": id,
                "hostname": hostname,
                "user": user,
                "memory": memory,
                "swap": swap,
                "cpu_common": cpu_common,
                "cpu_cores": cpu_cores,
                "kernel_version": kernel_version,
                "kernel_flags": kernel_flags,
                "kernel_extra": kernel_extra,
            })),
        )
        .await?;

        Ok(id)
    }

    async fn clickhouse_insert(
        url: &Url,
        web_client: &Client,
        table: &str,
        rows: impl Iterator<Item = Value> + Sync + Send + 'static,
    ) -> Result<()> {
        debug!("inserting into table {}", table);
        let mut url = url.clone();
        url.query_pairs_mut().append_pair(
            "query",
            &format!("INSERT INTO {} FORMAT JSONEachRow", table),
        );
        let rows = rows.map(|value| {
            let mut string = serde_json::to_string(&value)?;
            string.push('\n');
            Ok(string)
        });
        let rows = stream::iter(rows);
        let response = web_client
            .post(url)
            .body(Body::wrap_stream(rows))
            .send()
            .await?;

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

    fn save_to_file<E, W: Fn(File, &Stats) -> Result<(), E>>(
        paths: &[PathBuf],
        write: W,
        stats: &Stats,
    ) -> Result<()>
    where
        anyhow::Error: From<E>,
    {
        for path in paths {
            info!(
                "writing stats to {:?} using {:?}",
                path,
                std::any::type_name::<W>()
            );
            let file = File::create(path)?;
            write(file, stats)?;
        }
        Ok(())
    }
}
