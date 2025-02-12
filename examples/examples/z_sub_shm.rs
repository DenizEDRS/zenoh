use std::borrow::Cow;
use clap::Parser;
#[cfg(all(feature = "shared-memory", feature = "unstable"))]
use zenoh::shm::{zshm, zshmmut};
use zenoh::{bytes::ZBytes, config::Config, key_expr::KeyExpr};
use zenoh_examples::CommonArgs;
use std::time::{SystemTime, UNIX_EPOCH};
use std::io::Write;
#[tokio::main]
async fn main() {
    // Initiate logging
    zenoh::init_log_from_env_or("error");

    let (config, key_expr) = parse_args();

    println!("Opening session...");
    let session = zenoh::open(config).await.unwrap();

    println!("Declaring Subscriber on '{}'...", &key_expr);
    let subscriber = session.declare_subscriber(&key_expr).await.unwrap();

    println!("Press CTRL-C to quit...");
    while let Ok(mut sample) = subscriber.recv_async().await {
        let key_str = sample.key_expr().as_str().to_owned();

        // Get the raw byte size of the payload
        let payload_size = sample.payload().len();
        
        // Convert the payload into a string and detect its underlying type (SHM or RAW)
        let (payload_type, payload) = handle_bytes(sample.payload_mut());

        // Get the current (retrieval) timestamp in nanoseconds.
        let retrieval_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos();

        // Parse the publication timestamp from the payload.
        // The publisher creates payloads with a prefix like:
        // "[Time: %{now_ns}% ns] [{idx:4}] ..."
        if let Some(publication_timestamp) = parse_pub_timestamp(&payload) {
            println!("Key: {key_str}");
            println!("BufferType: {payload_type}");
            println!("Payload Size: {payload_size} bytes");
            println!("Publication Timestamp: {publication_timestamp} ns");
            println!("Retrieval Timestamp: {retrieval_timestamp} ns");

            let latency = retrieval_timestamp.saturating_sub(publication_timestamp);
            println!("Latency: {latency} ns");
            
            // Append timestamps and latency to a file
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("latency.txt")
                .unwrap();
            
            let data = format!(
                "{retrieval_timestamp}%{publication_timestamp}%{payload_size}\n"
            );
        file.write_all(data.as_bytes()).unwrap(); // Writing enabled here	
            // file.write_all(data.as_bytes()).unwrap(); // Uncomment to actually write
        } else {
            println!("Key: {key_str}");
            println!("BufferType: {payload_type}");
            println!("Payload Size: {payload_size} bytes");
            println!("No valid publication timestamp found in payload: '{payload}'");
        }

        println!();
    }
}

#[derive(clap::Parser, Clone, PartialEq, Eq, Hash, Debug)]
struct SubArgs {
    /// The Key Expression to subscribe to.
    #[arg(short, long, default_value = "demo/example/zenoh-rs-pub")]
    key: KeyExpr<'static>,
    #[command(flatten)]
    common: CommonArgs,
}

fn parse_args() -> (Config, KeyExpr<'static>) {
    let args = SubArgs::parse();
    (args.common.into(), args.key)
}

/// Attempts to parse the publication timestamp from a payload formatted as:
///
///     "[Time: %{timestamp}% ns] [{idx:4}] <rest of message>"
///
/// It returns `Some(timestamp)` (as nanoseconds) if parsing is successful.
fn parse_pub_timestamp(payload: &str) -> Option<u128> {
    let prefix = "[Time: %";
    let suffix = "% ns]";
    // Limit to the first 40 characters
    let limited_payload = &payload.chars().take(40).collect::<String>();
    let start_idx = limited_payload.find(prefix)? + prefix.len();
    let end_idx = limited_payload[start_idx..].find(suffix)? + start_idx;
    let timestamp_str = limited_payload[start_idx..end_idx].trim();
    timestamp_str.parse::<u128>().ok()
}

/// Inspects the ZBytes instance to determine the underlying buffer type
/// (SHM or RAW) and returns a string representation along with the payload
/// as a Cow<str>.
fn handle_bytes(bytes: &mut ZBytes) -> (&'static str, Cow<str>) {
    // Determine buffer type for indication purposes
    #[cfg(not(feature = "shared-memory"))]
    let bytes_type = "RAW";

    #[cfg(all(feature = "shared-memory", not(feature = "unstable")))]
    let bytes_type = "UNKNOWN";

    #[cfg(all(feature = "shared-memory", feature = "unstable"))]
    let bytes_type = match bytes.as_shm_mut() {
        Some(shm) => match <&mut zshm as TryInto<&mut zshmmut>>::try_into(shm) {
            Ok(_shm_mut) => "SHM (MUT)",
            Err(_) => "SHM (IMMUT)",
        },
        None => "RAW",
    };

    // Convert the ZBytes into a string. If conversion fails, the error is used.
    let bytes_string = bytes
        .try_to_string()
        .unwrap_or_else(|e| e.to_string().into());

    (bytes_type, bytes_string)
}
