CREATE TABLE IF NOT EXISTS host_info
(
    id UUID,
    hostname LowCardinality(String),
    user LowCardinality(String),
    memory UInt64,
    swap UInt64,
    cpu_common Map(LowCardinality(String), LowCardinality(String)),
    cpu_cores Array(Map(LowCardinality(String), LowCardinality(String))),
    kernel_version LowCardinality(String),
    kernel_flags Array(LowCardinality(String)),
    kernel_extra LowCardinality(String)
)
ENGINE = MergeTree()
ORDER BY (id)
