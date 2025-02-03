use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use clap::Parser;

#[cfg(all(feature = "shared-memory", feature = "unstable"))]
use zenoh::shm::{zshm, zshmmut};
use zenoh::{bytes::ZBytes, config::Config, key_expr::KeyExpr};
use zenoh_examples::CommonArgs;

#[tokio::main]
async fn main() {
    zenoh::init_log_from_env_or("error");
    let (config, key_expr) = parse_args();

    println!("Opening session...");
    let session = zenoh::open(config).await.unwrap();

    println!("Declaring Subscriber on '{}'...", &key_expr);
    let subscriber = session.declare_subscriber(&key_expr).await.unwrap();

    let file_path = "received_messages.txt";
    let counter = Arc::new(AtomicUsize::new(0));
    let max_messages = 5000;
    let mut received_data = Vec::with_capacity(max_messages);

    println!("Press CTRL-C to quit...");

    while let Ok(mut sample) = subscriber.recv_async().await {
        let count = counter.fetch_add(1, Ordering::Relaxed);
        if count >= max_messages {
            println!("Received {} messages. Exiting.", max_messages);
            break;
        }

        let timestamp_received = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos();

        let (_payload_type, payload) = handle_bytes(sample.payload_mut());
        let payload_size = payload.len(); // Payload size in bytes

        if let Some(mut timestamp_payload) = extract_timestamp(&payload) {
            timestamp_payload = timestamp_payload.trim();
            received_data.push(format!("{},{},{}", timestamp_received, timestamp_payload, payload_size));

            println!(
                ">> [Subscriber] Received message {}: Timestamp={}, PayloadSize={} bytes",
                count + 1,
                timestamp_received,
                payload_size
            );
        }
    }

    println!("Writing data to file...");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .expect("Unable to open file");
    let mut writer = BufWriter::new(file);
    for line in received_data {
        writeln!(writer, "{}", line).expect("Unable to write to file");
    }
    writer.flush().expect("Failed to flush buffer");
    println!("Finished receiving messages. Data logged to {}", file_path);
}

#[derive(clap::Parser, Clone, PartialEq, Eq, Hash, Debug)]
struct SubArgs {
    #[arg(short, long, default_value = "demo/example/**")]
    key: KeyExpr<'static>,
    #[command(flatten)]
    common: CommonArgs,
}

fn parse_args() -> (Config, KeyExpr<'static>) {
    let args = SubArgs::parse();
    (args.common.into(), args.key)
}

fn handle_bytes(bytes: &mut ZBytes) -> (&str, Cow<str>) {
    let bytes_type = {
        #[cfg(not(feature = "shared-memory"))]
        {
            "RAW"
        }
        #[cfg(all(feature = "shared-memory", not(feature = "unstable")))]
        {
            "UNKNOWN"
        }
        #[cfg(all(feature = "shared-memory", feature = "unstable"))]
        match bytes.as_shm_mut() {
            Some(shm) => match <&mut zshm as TryInto<&mut zshmmut>>::try_into(shm) {
                Ok(_shm_mut) => "SHM (MUT)",
                Err(_) => "SHM (IMMUT)",
            },
            None => "RAW",
        }
    };
    let bytes_string = bytes.try_to_string().unwrap_or_else(|e| e.to_string().into());
    (bytes_type, bytes_string)
}

fn extract_timestamp(payload: &str) -> Option<&str> {
    let start_marker = "[Timestamp:";
    let end_marker = "]";
    let start_pos = payload.find(start_marker)? + start_marker.len();
    let end_slice = &payload[start_pos..];
    let end_pos_relative = end_slice.find(end_marker)?;
    let end_pos = start_pos + end_pos_relative;
    Some(end_slice[..end_pos_relative].trim())
}
