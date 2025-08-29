CREATE TABLE IF NOT EXISTS replicas
(
    run_id UUID,
    replica_id UInt64 CODEC(DoubleDelta),
    replica_info UUID,
    ticks_per_second UInt64
)
ENGINE = MergeTree()
ORDER BY (run_id, replica_id)
