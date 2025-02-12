use clap::Parser;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zenoh::{
    key_expr::KeyExpr,
    shm::{
        BlockOn, GarbageCollect, PosixShmProviderBackend, ShmProviderBuilder, POSIX_PROTOCOL_ID,
    },
    Config, Wait,
};
use zenoh_examples::CommonArgs;

#[tokio::main]
async fn main() -> zenoh::Result<()> {
    // Initiate logging
    zenoh::init_log_from_env_or("error");

    // Parse arguments: config, key, user-string payload, payload size, shm size
    let (config, path, user_payload_str, payload_size, shm_size) = parse_args();

    println!("Opening session...");
    let session = zenoh::open(config).await.unwrap();

    println!("Creating POSIX SHM provider...");
    // 1. Create an SHM backend with user-defined total size (as `usize`)
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
    let publisher = session.declare_publisher(&path).await.unwrap();

    // Pre-generate a data buffer of 'payload_size' bytes (fill with 0xAB)
    let data_buffer = vec![b'x'; payload_size];
    let user_str_bytes = user_payload_str.as_bytes();
    let user_str_len = user_str_bytes.len();

    // 4. Calculate how large each SHM allocation needs to be.
    //    We'll store a text prefix + user text + binary data in the same buffer.
    //
    //    1024 is an arbitrary overhead for the prefix. If your prefix grows,
    //    increase it accordingly.
    let layout_size = 1024 + user_str_len + payload_size;

    println!(
        "Allocating Shared Memory Buffer Layout: {} bytes (payload_size={})",
        layout_size, payload_size
    );

    // 5. Create an allocation layout for repeated SHM allocations
    let layout = provider.alloc(layout_size).into_layout().unwrap();

    println!("Press CTRL-C to quit...");
    for idx in 0..u32::MAX {
        // Sleep for 100 microseconds between iterations
        tokio::time::sleep(Duration::from_micros(500)).await;

        // Get the current timestamp in nanoseconds
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos();

        // 6. Allocate an SHM buffer using the pre-created layout
        let mut sbuf = layout
            .alloc()
            .with_policy::<BlockOn<GarbageCollect>>()
            .await
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
            "Put SHM Data ('{}': '{}...' [total {} bytes])",
            path,
            String::from_utf8_lossy(&sbuf[0..std::cmp::min(slice_len, 128)]),
            slice_len
        );
        publisher.put(sbuf).await?;
    }

    Ok(())
}

// ----------------------------------------------------------------------------

#[derive(clap::Parser, Clone, PartialEq, Eq, Hash, Debug)]
struct Args {
    /// Key Expression on which to publish.
    #[arg(short, long, default_value = "demo/example/zenoh-rs-pub")]
    key: KeyExpr<'static>,

    /// A small user string to embed before the large data buffer.
    #[arg(short, long, default_value = "Pub from Rust SHM!")]
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
