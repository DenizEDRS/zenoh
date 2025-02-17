use std::borrow::Cow;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
#[cfg(all(feature = "shared-memory", feature = "unstable"))]

use zenoh::shm::{zshm, zshmmut};
use zenoh::{key_expr::KeyExpr};
use zenoh_examples::CommonArgs;
use zenoh::{
    bytes::ZBytes,
    key_expr::keyexpr,
    qos::CongestionControl,
    shm::{        BlockOn, GarbageCollect, PosixShmProviderBackend, ShmProviderBuilder, POSIX_PROTOCOL_ID,
    },
    Config, Wait,
};

use std::fs::OpenOptions;
use std::io::Write;

fn main() -> zenoh::Result<()>{




    zenoh::init_log_from_env_or("error");
    
    let (config, _key_expr) = parse_args();
    


    let user_payload_str = "Hello Zenoh SHM!";
    let shm_size: usize = 2000000000; // 
    let payload_size: usize = 500000000; // 

    let key_str = "demo/example/MAILBOX";
    let key_expr = unsafe { KeyExpr::from_str_unchecked(key_str) };
    
    let key_str_echo = "demo/example/ECHO";
    let key_expr_echo = unsafe { KeyExpr::from_str_unchecked(key_str_echo) };

    println!("Opening session...");
    let session = zenoh::open(config).wait().unwrap();


    println!("Creating POSIX SHM provider...");
    let backend = PosixShmProviderBackend::builder()
    .with_size(shm_size)      // <-- must be usize
    .unwrap()
    .wait()
    .unwrap();

    // 2. Create an SHM provider
    let provider = ShmProviderBuilder::builder()
    .protocol_id::<POSIX_PROTOCOL_ID>()
    .backend(backend)
    .wait();


    
    let subscriber = session.declare_subscriber(&key_expr).wait().unwrap();
    let ack_publisher =  session.declare_publisher(&key_expr_echo).wait().unwrap();

    let user_str_bytes = user_payload_str.as_bytes();
    let user_str_len = user_str_bytes.len();
    let layout_size: usize = 1024 + user_str_len + payload_size;

    println!(
        "Allocating Shared Memory Buffer Layout: {} bytes (payload_size={})",
        layout_size, payload_size
    );

    // 5. Create an allocation layout for repeated SHM allocations
    let layout = provider.alloc(layout_size).into_layout().unwrap();


    println!("Press CTRL-C to quit...");
    while let Ok(mut sample) = subscriber.recv() {

        let retrieval_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos();


        let key_str = sample.key_expr().as_str().to_owned();

        // Get the raw byte size of the payload
        let payload_size = sample.payload().len();
        
        // Convert the payload into a string and detect its underlying type (SHM or RAW)
        let (payload_type, payload) = handle_bytes(sample.payload_mut());

        
        let mut sbuf = layout
        .alloc()
        .with_policy::<BlockOn<GarbageCollect>>()
        .wait()
        .unwrap();


        let len = std::cmp::min(payload.len(), sbuf.len());
        sbuf[..len].copy_from_slice(&payload.as_bytes()[..len]);




        //copy the entire payload and send it tothe echo key
        println!("Publishing ACK to ECHO: {}", key_expr_echo);
        ack_publisher.put(sbuf).wait()?;
        println!(
            "Put [path: {},  total: {} bytes]",
            key_expr_echo,
            payload.len()
        );

        

        if let Some(publication_timestamp) = parse_pub_timestamp(&payload) {
            println!("Key: {key_str}");
            println!("BufferType: {payload_type}");
            println!("Payload Size: {payload_size} bytes");
            println!("Publication Timestamp: {publication_timestamp} ns");
            println!("Retrieval Timestamp: {retrieval_timestamp} ns");

            let latency = retrieval_timestamp.saturating_sub(publication_timestamp);
            println!("Latency: {latency} ns");


            //open a latency.txt, write in format: retrieval_timestamp, publication_timestamp
            let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("latency.txt")
            .unwrap();
            let latency_str = format!("{}, {}\n", retrieval_timestamp, publication_timestamp);
            file.write_all(latency_str.as_bytes()).unwrap();

        } else {
            println!("Key: {key_str}");
            println!("BufferType: {payload_type}");
            println!("Payload Size: {payload_size} bytes");
            println!("No valid publication timestamp found in payload: '{payload}'");
        }

        println!();
    }
    Ok(())
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
/// It returns Some(timestamp) (as nanoseconds) if parsing is successful.
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
