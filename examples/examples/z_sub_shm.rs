use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::Write;
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
    // Initiate logging
    zenoh::init_log_from_env_or("error");

    let (config, key_expr) = parse_args();

    println!("Opening session...");
    let session = zenoh::open(config).await.unwrap();

    println!("Declaring Subscriber on '{}'...", &key_expr);
    let subscriber = session.declare_subscriber(&key_expr).await.unwrap();

    // File to log timestamp and payload
    let file_path = "received_messages.csv";
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .expect("Unable to open file");

    // Write CSV header if file is empty
    if file.metadata().unwrap().len() == 0 {
        writeln!(file, "timestamp_received,messagePayload").unwrap();
    }

    // Atomic counter for tracking messages
    let counter = Arc::new(AtomicUsize::new(0));
    let max_messages = 5000;

    println!("Press CTRL-C to quit...");
    while let Ok(mut sample) = subscriber.recv_async().await {
        // Increment the counter
        let count = counter.fetch_add(1, Ordering::Relaxed);

        if count >= max_messages {
            println!("Received {} messages. Exiting.", max_messages);
            break;
        }

        // Get the current timestamp
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos();

        // Handle payload and get its string representation
        let (_payload_type, payload) = handle_bytes(sample.payload_mut());

        // Write the timestamp and payload to the CSV file
        writeln!(file, "{},{}", timestamp, payload).unwrap();

        println!(
            ">> [Subscriber] Received message {} with timestamp {} and payload '{}'",
            count + 1,
            timestamp,
            payload
        );
    }

    println!("Finished receiving messages. Data logged to {}", file_path);
}

#[derive(clap::Parser, Clone, PartialEq, Eq, Hash, Debug)]
struct SubArgs {
    #[arg(short, long, default_value = "demo/example/**")]
    /// The Key Expression to subscribe to.
    key: KeyExpr<'static>,
    #[command(flatten)]
    common: CommonArgs,
}

fn parse_args() -> (Config, KeyExpr<'static>) {
    let args = SubArgs::parse();
    (args.common.into(), args.key)
}

fn handle_bytes(bytes: &mut ZBytes) -> (&str, Cow<str>) {
    // Determine buffer type for indication purpose
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

    let bytes_string = bytes
        .try_to_string()
        .unwrap_or_else(|e| e.to_string().into());

    (bytes_type, bytes_string)
}
