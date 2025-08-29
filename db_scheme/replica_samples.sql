CREATE TABLE IF NOT EXISTS replica_samples
(
    run_id UUID,
    replica_id UInt64,
    sample_id UInt64 CODEC(DoubleDelta),
    timestamp UInt64,
    memory_usage_in_bytes UInt64,
    memory_used_by_stats UInt64,
    user_mode_ticks UInt64,
    kernel_mode_ticks UInt64,
    bytes_rx UInt64,
    bytes_tx UInt64,
    packets_rx UInt64,
    packets_tx UInt64,
    messages_algo_unicast_rx UInt64,
    messages_algo_unicast_tx UInt64,
    messages_algo_broadcast_rx UInt64,
    messages_algo_broadcast_tx UInt64,
    messages_server_unicast_rx UInt64,
    messages_server_unicast_tx UInt64,
    messages_server_broadcast_rx UInt64,
    messages_server_broadcast_tx UInt64
)
ENGINE = MergeTree()
ORDER BY (run_id, replica_id, sample_id)
