//! Binary entrypoint — delegates to the `db_partition_mgmt` library.
//! All logic lives in `lib.rs` so integration tests can exercise it.

use lambda_runtime::{Error, service_fn};

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    lambda_runtime::run(service_fn(db_partition_mgmt::handler)).await
}
