//! WIT metadata construction using embedded WIT files.
//! Works in wasm32/browser environments where std::fs is unavailable.
//!
//! Mirrors the native `push_dir` builders in wasi_http.rs:
//! - `build_http_wit_metadata_embedded`   → world `simple-http`
//! - `build_combined_wit_metadata_embedded` → world `outlayer-http` (P2 components)
//!
//! NOTE: the outlayer-nohttp world has no embedded equivalent — it is dead code
//! on the native side too (build_combined_p2_component always uses outlayer-http).

use wit_parser::SourceMap;

// ── wasi deps (pulled in by every world) ──
const DEPS_CLI_COMMAND_WIT: &str = include_str!("../wit/deps/cli/command.wit");
const DEPS_CLI_ENVIRONMENT_WIT: &str = include_str!("../wit/deps/cli/environment.wit");
const DEPS_CLI_EXIT_WIT: &str = include_str!("../wit/deps/cli/exit.wit");
const DEPS_CLI_IMPORTS_WIT: &str = include_str!("../wit/deps/cli/imports.wit");
const DEPS_CLI_RUN_WIT: &str = include_str!("../wit/deps/cli/run.wit");
const DEPS_CLI_STDIO_WIT: &str = include_str!("../wit/deps/cli/stdio.wit");
const DEPS_CLI_TERMINAL_WIT: &str = include_str!("../wit/deps/cli/terminal.wit");
const DEPS_CLOCKS_MONOTONIC_CLOCK_WIT: &str =
    include_str!("../wit/deps/clocks/monotonic-clock.wit");
const DEPS_CLOCKS_TIMEZONE_WIT: &str = include_str!("../wit/deps/clocks/timezone.wit");
const DEPS_CLOCKS_WALL_CLOCK_WIT: &str = include_str!("../wit/deps/clocks/wall-clock.wit");
const DEPS_CLOCKS_WORLD_WIT: &str = include_str!("../wit/deps/clocks/world.wit");
const DEPS_FILESYSTEM_PREOPENS_WIT: &str = include_str!("../wit/deps/filesystem/preopens.wit");
const DEPS_FILESYSTEM_TYPES_WIT: &str = include_str!("../wit/deps/filesystem/types.wit");
const DEPS_FILESYSTEM_WORLD_WIT: &str = include_str!("../wit/deps/filesystem/world.wit");
const DEPS_HTTP_HANDLER_WIT: &str = include_str!("../wit/deps/http/handler.wit");
const DEPS_HTTP_PROXY_WIT: &str = include_str!("../wit/deps/http/proxy.wit");
const DEPS_HTTP_TYPES_WIT: &str = include_str!("../wit/deps/http/types.wit");
const DEPS_IO_ERROR_WIT: &str = include_str!("../wit/deps/io/error.wit");
const DEPS_IO_POLL_WIT: &str = include_str!("../wit/deps/io/poll.wit");
const DEPS_IO_STREAMS_WIT: &str = include_str!("../wit/deps/io/streams.wit");
const DEPS_IO_WORLD_WIT: &str = include_str!("../wit/deps/io/world.wit");
const DEPS_RANDOM_INSECURE_SEED_WIT: &str =
    include_str!("../wit/deps/random/insecure-seed.wit");
const DEPS_RANDOM_INSECURE_WIT: &str = include_str!("../wit/deps/random/insecure.wit");
const DEPS_RANDOM_RANDOM_WIT: &str = include_str!("../wit/deps/random/random.wit");
const DEPS_RANDOM_WORLD_WIT: &str = include_str!("../wit/deps/random/world.wit");
const DEPS_SOCKETS_INSTANCE_NETWORK_WIT: &str =
    include_str!("../wit/deps/sockets/instance-network.wit");
const DEPS_SOCKETS_IP_NAME_LOOKUP_WIT: &str =
    include_str!("../wit/deps/sockets/ip-name-lookup.wit");
const DEPS_SOCKETS_NETWORK_WIT: &str = include_str!("../wit/deps/sockets/network.wit");
const DEPS_SOCKETS_TCP_CREATE_SOCKET_WIT: &str =
    include_str!("../wit/deps/sockets/tcp-create-socket.wit");
const DEPS_SOCKETS_TCP_WIT: &str = include_str!("../wit/deps/sockets/tcp.wit");
const DEPS_SOCKETS_UDP_CREATE_SOCKET_WIT: &str =
    include_str!("../wit/deps/sockets/udp-create-socket.wit");
const DEPS_SOCKETS_UDP_WIT: &str = include_str!("../wit/deps/sockets/udp.wit");
const DEPS_SOCKETS_WORLD_WIT: &str = include_str!("../wit/deps/sockets/world.wit");

// ── outlayer / near custom deps (imported by world outlayer-http) ──
const DEPS_OUTLAYER_API_HOST_WIT: &str = include_str!("../wit/deps/outlayer-api/host.wit");
const DEPS_NEAR_PAYMENT_WIT: &str = include_str!("../wit/deps/near-payment/payment.wit");
const DEPS_NEAR_VRF_WIT: &str = include_str!("../wit/deps/near-vrf/vrf.wit");
const DEPS_OUTLAYER_WALLET_WIT: &str = include_str!("../wit/deps/outlayer-wallet/wallet.wit");
const DEPS_NEAR_RPC_WIT: &str = include_str!("../wit/deps/near-rpc/rpc.wit");
const DEPS_NEAR_STORAGE_WIT: &str = include_str!("../wit/deps/near-storage/storage.wit");

// ── worlds ──
const SIMPLE_HTTP_WIT: &str = include_str!("../wit/deps/simple-http/simple-http.wit");
const COMBINED_WIT: &str = include_str!("../wit/deps/combined.wit");

/// Build WIT metadata using embedded WIT files (no filesystem access):
/// world `simple-http` (parity with native `build_http_wit_metadata`).
pub fn build_http_wit_metadata_embedded(
) -> Result<(wit_parser::Resolve, wit_parser::WorldId), String> {
    let mut resolve = build_deps()?;
    push_world(&mut resolve, &[("simple-http.wit", SIMPLE_HTTP_WIT)], "simple-http")?;
    let world = find_world(&resolve, &["simple-http"])?;
    Ok((resolve, world))
}

/// Build WIT metadata using embedded WIT files (no filesystem access):
/// world `outlayer-http` (parity with native `build_combined_wit_metadata`).
/// This is the world used to componentize P2 cores — it carries the
/// near:storage / near:rpc / near:payment / near:vrf / outlayer:api /
/// outlayer:wallet imports alongside the full wasi surface.
pub fn build_combined_wit_metadata_embedded(
) -> Result<(wit_parser::Resolve, wit_parser::WorldId), String> {
    let mut resolve = build_deps()?;
    push_world(&mut resolve, &[("combined.wit", COMBINED_WIT)], "outlayer-http")?;
    let world = find_world(&resolve, &["outlayer-http"])?;
    Ok((resolve, world))
}

/// Push every dependency package (same order as the native builders:
/// io has no deps, everything else depends on it; custom near-*/outlayer-*
/// packages come after the wasi set, as in `push_dir` order).
fn build_deps() -> Result<wit_parser::Resolve, String> {
    let mut resolve = wit_parser::Resolve::new();

    let groups: &[&[(&str, &str)]] = &[
        &[
            ("deps/io/error.wit", DEPS_IO_ERROR_WIT),
            ("deps/io/poll.wit", DEPS_IO_POLL_WIT),
            ("deps/io/streams.wit", DEPS_IO_STREAMS_WIT),
            ("deps/io/world.wit", DEPS_IO_WORLD_WIT),
        ],
        &[
            ("deps/clocks/monotonic-clock.wit", DEPS_CLOCKS_MONOTONIC_CLOCK_WIT),
            ("deps/clocks/timezone.wit", DEPS_CLOCKS_TIMEZONE_WIT),
            ("deps/clocks/wall-clock.wit", DEPS_CLOCKS_WALL_CLOCK_WIT),
            ("deps/clocks/world.wit", DEPS_CLOCKS_WORLD_WIT),
        ],
        &[
            ("deps/random/insecure-seed.wit", DEPS_RANDOM_INSECURE_SEED_WIT),
            ("deps/random/insecure.wit", DEPS_RANDOM_INSECURE_WIT),
            ("deps/random/random.wit", DEPS_RANDOM_RANDOM_WIT),
            ("deps/random/world.wit", DEPS_RANDOM_WORLD_WIT),
        ],
        &[
            ("deps/filesystem/preopens.wit", DEPS_FILESYSTEM_PREOPENS_WIT),
            ("deps/filesystem/types.wit", DEPS_FILESYSTEM_TYPES_WIT),
            ("deps/filesystem/world.wit", DEPS_FILESYSTEM_WORLD_WIT),
        ],
        &[
            ("deps/sockets/instance-network.wit", DEPS_SOCKETS_INSTANCE_NETWORK_WIT),
            ("deps/sockets/ip-name-lookup.wit", DEPS_SOCKETS_IP_NAME_LOOKUP_WIT),
            ("deps/sockets/network.wit", DEPS_SOCKETS_NETWORK_WIT),
            ("deps/sockets/tcp-create-socket.wit", DEPS_SOCKETS_TCP_CREATE_SOCKET_WIT),
            ("deps/sockets/tcp.wit", DEPS_SOCKETS_TCP_WIT),
            ("deps/sockets/udp-create-socket.wit", DEPS_SOCKETS_UDP_CREATE_SOCKET_WIT),
            ("deps/sockets/udp.wit", DEPS_SOCKETS_UDP_WIT),
            ("deps/sockets/world.wit", DEPS_SOCKETS_WORLD_WIT),
        ],
        &[
            ("deps/cli/command.wit", DEPS_CLI_COMMAND_WIT),
            ("deps/cli/environment.wit", DEPS_CLI_ENVIRONMENT_WIT),
            ("deps/cli/exit.wit", DEPS_CLI_EXIT_WIT),
            ("deps/cli/imports.wit", DEPS_CLI_IMPORTS_WIT),
            ("deps/cli/run.wit", DEPS_CLI_RUN_WIT),
            ("deps/cli/stdio.wit", DEPS_CLI_STDIO_WIT),
            ("deps/cli/terminal.wit", DEPS_CLI_TERMINAL_WIT),
        ],
        &[
            ("deps/http/handler.wit", DEPS_HTTP_HANDLER_WIT),
            ("deps/http/proxy.wit", DEPS_HTTP_PROXY_WIT),
            ("deps/http/types.wit", DEPS_HTTP_TYPES_WIT),
        ],
        // custom packages, native push_dir order — ONE GROUP PER PACKAGE:
        // rpc.wit defines a world importing the other four, and a SourceMap
        // group can only resolve foreign imports against ALREADY-PUSHED
        // packages (push_dir semantics), so they must not share a group.
        &[
            ("deps/outlayer-api/host.wit", DEPS_OUTLAYER_API_HOST_WIT),
        ],
        &[
            ("deps/near-payment/payment.wit", DEPS_NEAR_PAYMENT_WIT),
        ],
        &[
            ("deps/near-vrf/vrf.wit", DEPS_NEAR_VRF_WIT),
        ],
        &[
            ("deps/outlayer-wallet/wallet.wit", DEPS_OUTLAYER_WALLET_WIT),
        ],
        &[
            ("deps/near-rpc/rpc.wit", DEPS_NEAR_RPC_WIT),
        ],
        &[
            ("deps/near-storage/storage.wit", DEPS_NEAR_STORAGE_WIT),
        ],
        // NOTE: world files are NOT pushed here — each builder pushes exactly
        // one world (simple-http.wit or combined.wit); pushing a package twice
        // makes push_group fail with a duplicate-package error.
    ];

    for group in groups {
        let pkg = build_group(group)?;
        resolve
            .push_group(pkg)
            .map_err(|e| format!("push_group failed: {e:?}"))?;
    }
    Ok(resolve)
}

fn push_world(
    resolve: &mut wit_parser::Resolve,
    files: &[(&str, &str)],
    _label: &str,
) -> Result<(), String> {
    let pkg = build_group(files)?;
    resolve
        .push_group(pkg)
        .map_err(|e| format!("push_group world: {e:?}"))?;
    Ok(())
}

fn find_world(
    resolve: &wit_parser::Resolve,
    names: &[&str],
) -> Result<wit_parser::WorldId, String> {
    for (_pkg_id, pkg) in resolve.packages.iter() {
        for (name, world_id) in &pkg.worlds {
            if names.contains(&name.as_str()) {
                return Ok(*world_id);
            }
        }
    }
    Err(format!("world {} not found", names[0]))
}

fn build_group(files: &[(&str, &str)]) -> Result<wit_parser::UnresolvedPackageGroup, String> {
    let mut map = SourceMap::default();
    for &(path, contents) in files {
        // wit-parser 0.244: push takes a &Path (push_str is 0.248-only) and
        // parse returns a plain anyhow Result (no (map, error) tuple).
        map.push(std::path::Path::new(path), contents);
    }
    map.parse().map_err(|e| format!("WIT parse error: {e}"))
}
