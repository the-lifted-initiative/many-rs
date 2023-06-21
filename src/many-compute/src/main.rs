use clap::Parser;
use many_identity::verifiers::AnonymousVerifier;
use many_identity::Address;
use many_identity_dsa::{CoseKeyIdentity, CoseKeyVerifier};
use many_modules::{abci_backend, compute, events};
use many_server::transport::http::HttpServer;
use many_server::ManyServer;
use many_server_cache::{RequestCacheValidator, RocksDbCacheBackend};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{debug, info};
use many_protocol::ManyUrl;

mod error;
mod module;
mod storage;
mod validator;

use module::*;
use validator::*;

#[derive(Debug, Parser)]
struct Opts {
    #[clap(flatten)]
    common_flags: many_cli_helpers::CommonCliFlags,

    /// The location of a PEM file for the identity of this server.
    #[clap(long)]
    pem: PathBuf,

    /// The address and port to bind to for the MANY Http server.
    #[clap(long, short, default_value = "127.0.0.1:8000")]
    addr: SocketAddr,

    /// Uses an ABCI application module.
    #[clap(long)]
    abci: bool,

    /// Path of a state file (that will be used for the initial setup).
    #[clap(long)]
    state: Option<PathBuf>,

    /// Path to a persistent store database (rocksdb).
    #[clap(long)]
    persistent: PathBuf,

    /// Delete the persistent storage to start from a clean state.
    /// If this is not specified the initial state will not be used.
    #[clap(long, short)]
    clean: bool,

    /// Path to a JSON file containing an array of MANY addresses
    /// Only addresses from this array will be able to execute commands, e.g., send, put, ...
    /// Any addresses will be able to execute queries, e.g., balance, get, ...
    #[clap(long)]
    allow_addrs: Option<PathBuf>,

    /// Database path to the request cache to validate duplicate messages.
    /// If unspecified, the server will not verify transactions for duplicate
    /// messages.
    #[clap(long)]
    cache_db: Option<PathBuf>,

    #[clap(long)]
    whitelist: Option<PathBuf>,

    #[clap(flatten)]
    akash_opt: AkashOpt,
}

#[derive(Debug, Parser)]
pub struct AkashOpt {
    #[clap(long, default_value = "akashnet-2")]
    akash_chain_id: String,

    // Akash needs the port number in the url even if the schema is known.
    // Unfortunately, the `url` crate drops the port number from the serialization when the schema is known.
    // TODO: Make `ManyUrl` a real wrapper with a `to_string_with_port` method.
    #[clap(long, default_value = "https://rpc.akashnet.net:443")]
    akash_rpc: String,

    #[clap(long, default_value = "auto")]
    akash_gas: String,

    #[clap(long, default_value = "1.25")]
    akash_gas_adjustment: f64,

    #[clap(long, default_value = "0.025uakt")]
    akash_gas_price: String,

    #[clap(long, default_value = "amino-json")]
    akash_sign_mode: String,

    #[clap(long, default_value = "")]
    akash_wallet: String,
}

fn main() {
    let Opts {
        common_flags,
        pem,
        addr,
        abci,
        mut state,
        persistent,
        clean,
        allow_addrs,
        cache_db,
        whitelist,
        akash_opt,
    } = Opts::parse();


    common_flags.init_logging().unwrap();

    debug!("{:?}", Opts::parse());
    info!(
        version = std::env!("CARGO_PKG_VERSION"),
        git_sha = std::env!("VERGEN_GIT_SHA")
    );
    // akash_opt.akash_rpc.port_or_known_default().expect("RPC node URL is missing port number.");

    if clean {
        // Delete the persistent storage.
        let _ = std::fs::remove_dir_all(persistent.as_path());
    } else if persistent.exists() {
        // Initial state is ignored.
        state = None;
    }

    let key = CoseKeyIdentity::from_pem(std::fs::read_to_string(pem).unwrap()).unwrap();

    let state = state.map(|state| {
        let content = std::fs::read_to_string(state).unwrap();
        json5::from_str(&content).unwrap()
    });

    let module = if persistent.exists() {
        if state.is_some() {
            tracing::warn!(
                r#"
                An existing persistent store {} was found and a staging file {state:?} was given.
                Ignoring staging file and loading existing persistent store.
                "#,
                persistent.display()
            );
        }

        ComputeModuleImpl::load(akash_opt, persistent, abci).unwrap()
    } else if let Some(state) = state {
        ComputeModuleImpl::new(state, akash_opt, persistent, abci).unwrap()
    } else {
        panic!("Persistent store or staging file not found.")
    };

    let module = Arc::new(Mutex::new(module));

    let many = ManyServer::simple(
        "many-compute",
        key,
        (AnonymousVerifier, CoseKeyVerifier),
        Some(env!("CARGO_PKG_VERSION").to_string()),
    );

    {
        let mut s = many.lock().unwrap();
        s.add_module(compute::ComputeModule::new(module.clone()));
        // let kvstore_command_module = kvstore::KvStoreCommandsModule::new(module.clone());
        // if let Some(path) = allow_addrs {
        //     let allow_addrs: BTreeSet<Address> =
        //         json5::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        //     s.add_module(allow_addrs::AllowAddrsModule {
        //         inner: kvstore_command_module,
        //         allow_addrs,
        //     });
        // } else {
        //     s.add_module(kvstore_command_module);
        // }
        // s.add_module(kvstore::KvStoreTransferModule::new(module.clone()));
        // s.add_module(events::EventsModule::new(module.clone()));

        if abci {
            s.set_timeout(u64::MAX);
            s.add_module(abci_backend::AbciModule::new(module));
        }

        if let Some(p) = cache_db {
            s.add_validator(RequestCacheValidator::new(RocksDbCacheBackend::new(p)));
        }

        if let Some(p) = whitelist {
            s.add_validator(WhitelistValidator::new(p));
        }
    }
    let mut many_server = HttpServer::new(many);

    signal_hook::flag::register(signal_hook::consts::SIGTERM, many_server.term_signal())
        .expect("Could not register signal handler");
    signal_hook::flag::register(signal_hook::consts::SIGHUP, many_server.term_signal())
        .expect("Could not register signal handler");
    signal_hook::flag::register(signal_hook::consts::SIGINT, many_server.term_signal())
        .expect("Could not register signal handler");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(many_server.bind(addr)).unwrap();
}