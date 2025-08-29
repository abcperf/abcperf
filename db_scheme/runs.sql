CREATE TABLE IF NOT EXISTS runs
(
    run_id UUID,
    seed String,
    orchestrator_info UUID,

    abcperf_version LowCardinality(String),
    bin_name LowCardinality(String),
    bin_version LowCardinality(String),
    experiment_label LowCardinality(String),
    run_tags Map(LowCardinality(String), LowCardinality(String)),
    raw_config LowCardinality(String),

    start_time DateTime('UTC'),
    end_time DateTime('UTC'),
    experiment_duration Float64,

    n UInt64,
    t UInt64,
    client_workers UInt64,

    abc_algorithm LowCardinality(String),
    state_machine_application LowCardinality(String)
)
ENGINE = MergeTree()
ORDER BY (run_id)
