//! Shared background loader threads for CPU decode, pack mounts, and relic batches.
//!
//! Priorities: P0 room decode (next scene) > P1 chain prefetch > P2 pack mount > P3 relic.

use std::sync::OnceLock;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, unbounded};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LoaderPriority {
    RoomDecode = 0,
    ChainPrefetch = 1,
    PackMount = 2,
    RelicBatch = 3,
}

type Job = Box<dyn FnOnce() + Send>;

struct LoaderPool {
    room_tx: Sender<Job>,
    chain_tx: Sender<Job>,
    pack_tx: Sender<Job>,
    relic_tx: Sender<Job>,
    _workers: Vec<JoinHandle<()>>,
}

static POOL: OnceLock<LoaderPool> = OnceLock::new();

fn pool() -> &'static LoaderPool {
    POOL.get_or_init(|| {
        let (room_tx, room_rx) = unbounded();
        let (chain_tx, chain_rx) = unbounded();
        let (pack_tx, pack_rx) = unbounded();
        let (relic_tx, relic_rx) = unbounded();
        let worker_count = std::env::var("MAHJURO_LOADER_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3)
            .clamp(1, 4);
        let mut workers = Vec::with_capacity(worker_count);
        for i in 0..worker_count {
            let room_rx = room_rx.clone();
            let chain_rx = chain_rx.clone();
            let pack_rx = pack_rx.clone();
            let relic_rx = relic_rx.clone();
            workers.push(
                thread::Builder::new()
                    .name(format!("mahjuro-loader-{i}"))
                    .spawn(move || worker_loop(room_rx, chain_rx, pack_rx, relic_rx))
                    .expect("loader pool worker"),
            );
        }
        LoaderPool {
            room_tx,
            chain_tx,
            pack_tx,
            relic_tx,
            _workers: workers,
        }
    })
}

fn worker_loop(
    room_rx: Receiver<Job>,
    chain_rx: Receiver<Job>,
    pack_rx: Receiver<Job>,
    relic_rx: Receiver<Job>,
) {
    loop {
        crossbeam_channel::select! {
            recv(room_rx) -> msg => {
                if let Ok(job) = msg { job(); }
            }
            recv(chain_rx) -> msg => {
                if let Ok(job) = msg { job(); }
            }
            recv(pack_rx) -> msg => {
                if let Ok(job) = msg { job(); }
            }
            recv(relic_rx) -> msg => {
                if let Ok(job) = msg { job(); }
            }
            default => thread::sleep(std::time::Duration::from_millis(2)),
        }
    }
}

fn submit(priority: LoaderPriority, job: Job) {
    let pool = pool();
    let tx = match priority {
        LoaderPriority::RoomDecode => &pool.room_tx,
        LoaderPriority::ChainPrefetch => &pool.chain_tx,
        LoaderPriority::PackMount => &pool.pack_tx,
        LoaderPriority::RelicBatch => &pool.relic_tx,
    };
    if tx.send(job).is_err() {
        log::error!("loader pool channel closed");
    }
}

/// Run a room GLB CPU decode (P0 — next-scene work).
pub fn submit_room_decode(job: impl FnOnce() + Send + 'static) {
    submit(LoaderPriority::RoomDecode, Box::new(job));
}

/// Hub chain prefetch decode (P1).
pub fn submit_chain_prefetch(job: impl FnOnce() + Send + 'static) {
    submit(LoaderPriority::ChainPrefetch, Box::new(job));
}

/// Lazy pack mount (P2).
pub fn submit_pack_mount(job: impl FnOnce() + Send + 'static) {
    submit(LoaderPriority::PackMount, Box::new(job));
}

/// Relic RLC2 batch decode (P3).
pub fn submit_relic_batch(job: impl FnOnce() + Send + 'static) {
    submit(LoaderPriority::RelicBatch, Box::new(job));
}
