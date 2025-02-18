use clap::Parser;
use std::borrow::Cow;
use std::time::{Duration,SystemTime, UNIX_EPOCH};
#[cfg(all(feature = "shared-memory", feature = "unstable"))]
use zenoh::{
    bytes::ZBytes,
    key_expr::KeyExpr,
    qos::CongestionControl,

    shm::{
        BlockOn, GarbageCollect, PosixShmProviderBackend, ShmProviderBuilder, POSIX_PROTOCOL_ID,
    },
    Config, Wait,
};

use std::fs::OpenOptions;
use std::io::Write;

use zenoh_examples::CommonArgs;
use zenoh::shm::zshm;
use zenoh::shm::zshmmut;

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



fn main() -> zenoh::Result<()> {
    // Initiate logging






    zenoh::init_log_from_env_or("error");

    // Parse arguments: config, key, user-string payload, payload size, shm size
    let (config, _path, _user_payload_str, payload_size, shm_size) = parse_args();

    let user_payload_str = "Hello Zenoh SHM!";






    let key_str = "demo/example/MAILBOX";
    let key_expr = unsafe { KeyExpr::from_str_unchecked(key_str) };

    let key_str_echo = "demo/example/ECHO";
    let key_expr_echo = unsafe { KeyExpr::from_str_unchecked(key_str_echo) };


    println!("Opening session...");
    let session = zenoh::open(config).wait().unwrap();

    println!("Creating POSIX SHM provider...");
    // 1. Create an SHM backend with user-defined total size (as usize)

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
    
    // 3. Declare the publisher
    let publisher = session.declare_publisher(&key_expr).wait().unwrap();
    let ack_subscriber = session.declare_subscriber(&key_expr_echo).wait().unwrap();

    // Pre-generate a data buffer of 'payload_size' bytes (fill with 0xAB)
    let data_buffer = vec![b'x'; payload_size];
    let user_str_bytes = user_payload_str.as_bytes();
    let user_str_len = user_str_bytes.len();
    let layout_size = 1024 + user_str_len + payload_size;

    println!(
        "Allocating Shared Memory Buffer Layout: {} bytes (payload_size={})",
        layout_size, payload_size
    );

    // 5. Create an allocation layout for repeated SHM allocations
    let layout = provider.alloc(layout_size).into_layout().unwrap();

    println!("Press CTRL-C to quit...");
    for idx in 0..u32::MAX {

        println!("############### Start of Procedure {} ###############", idx);

        
        // Get the current timestamp in nanoseconds
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos();

        // 6. Allocate an SHM buffer using the pre-created layout
        let mut sbuf = layout
            .alloc()
            .with_policy::<BlockOn<GarbageCollect>>()
            .wait()
            .unwrap();

        // Build a prefix string: timestamp + iteration index
        let prefix = format!("[Time: %{now_ns}% ns] [{idx:4}] ");
        let prefix_len = prefix.len();

        // 7. Copy prefix + user payload + binary data into the SHM buffer
        sbuf[0..prefix_len].copy_from_slice(prefix.as_bytes());

        let str_start = prefix_len;
        let str_end = str_start + user_str_len;
        sbuf[str_start..str_end].copy_from_slice(user_str_bytes);

        let data_start = str_end;
        let data_end = data_start + data_buffer.len();
        sbuf[data_start..data_end].copy_from_slice(&data_buffer[..]);

        // We'll publish the entire region we've written
        let slice_len = data_end;

        println!(
            "Put [path: {}, content preview: {}, total: {} bytes]",
            key_expr,
            String::from_utf8_lossy(&sbuf[0..std::cmp::min(slice_len, 128)]),
            slice_len
        );
        publisher.put(sbuf).wait()?;

        // 8. Wait for an ACK from the subscriber

        while let Ok(mut sample) = ack_subscriber.recv() {
            println!("Received ACK from ECHO key: {key_expr_echo}");
        
            let key_str = sample.key_expr().as_str().to_owned();
        
            let payload_size = sample.payload().len();
            
            let (payload_type, payload) = handle_bytes(sample.payload_mut());
        
            let retrieval_timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_nanos();
            
            println!("Key: {key_str}");
            println!("Payload Type: {payload_type}");
            println!("Payload Size: {payload_size} bytes");
            println!("############### End of Procedure ###############");
        
            break; // This breaks once we get an ACK, but logs all relevant data
        }
    
        //new timestamp
        let rtt_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos();

        //open a roundtrip.txt, write rtt_timestamp, now_ns
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("roundtrip.txt")
            .unwrap();
            let rtt_str = format!("{}, {}\n", rtt_timestamp, now_ns);
            file.write_all(rtt_str.as_bytes()).unwrap();
        
    }

    Ok(())
}

// ----------------------------------------------------------------------------

#[derive(clap::Parser, Clone, PartialEq, Eq, Hash, Debug)]
struct Args {
    /// Key Expression on which to publish.
    #[arg(short, long, default_value = "demo/example/MAILBOX")]
    key: KeyExpr<'static>,

    /// A small user string to embed before the large data buffer.
    #[arg(short, long, default_value = "Pub")]
    payload: String,

    /// The total size (in bytes) of the large binary data buffer.
    #[arg(long, default_value = "1024")]
    payload_size: usize,

    /// The total size (in bytes) of the shared memory region. Must be >= payload_size + overhead.
    #[arg(long, default_value = "10485760", help = "e.g. 10485760 = 10MB")]
    shm_size: usize,

    #[command(flatten)]
    common: CommonArgs,
}

fn parse_args() -> (Config, KeyExpr<'static>, String, usize, usize) {
    let args = Args::parse();
    (
        args.common.into(),
        args.key,
        args.payload,
        args.payload_size,
        args.shm_size,
    )
}


/*

fn main() {
    // Initiate logging
    zenoh::init_log_from_env_or("error");

    let (config, key_expr) = parse_args();

    println!("Opening session...");
    let session = zenoh::open(config).wait().unwrap();

    println!("Declaring Subscriber on '{}'...", &key_expr);
    
    let subscriber = session.declare_subscriber(&key_expr).wait().unwrap();

    println!("Press CTRL-C to quit...");
    while let Ok(mut sample) = subscriber.recv() {
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


        if let Some(publication_timestamp) = parse_pub_timestamp(&payload) {
            println!("Key: {key_str}");
            println!("BufferType: {payload_type}");
            println!("Payload Size: {payload_size} bytes");
            println!("Publication Timestamp: {publication_timestamp} ns");
            println!("Retrieval Timestamp: {retrieval_timestamp} ns");

            let latency = retrieval_timestamp.saturating_sub(publication_timestamp);
            println!("Latency: {latency} ns");

        } else {
            println!("Key: {key_str}");
            println!("BufferType: {payload_type}");
            println!("Payload Size: {payload_size} bytes");
            println!("No valid publication timestamp found in payload: '{payload}'");
        }

        println!();
    }
}

*/
