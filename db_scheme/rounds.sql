CREATE TABLE IF NOT EXISTS rounds
(
    run_id UUID,
    replica_id UInt64,
    round_id UInt64 CODEC(DoubleDelta),
    timestamp UInt64,
)
ENGINE = MergeTree()
ORDER BY (run_id, replica_id, round_id)
