CREATE TABLE IF NOT EXISTS client_samples
(
    run_id UUID,
    client_id UInt64,
    sample_id UInt64 CODEC(DoubleDelta),
    timestamp UInt64 CODEC(T64,ZSTD(1)),
    processing_time UInt64 CODEC(T64,ZSTD(1))
)
ENGINE = MergeTree()
ORDER BY (run_id, client_id, sample_id)
