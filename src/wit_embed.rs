//! WIT metadata construction using embedded WIT files.
//! Works in wasm32/browser environments where std::fs is unavailable.

use wit_parser::SourceMap;

/// Build WIT metadata using embedded WIT files (no filesystem access).
pub fn build_http_wit_metadata_embedded() -> Result<(wit_parser::Resolve, wit_parser::WorldId), String> {
    let mut resolve = wit_parser::Resolve::new();

    const WORLD_WIT: &str = include_str!("../wit/deps/simple-http/simple-http.wit");
    const DEPS_CLI_COMMAND_WIT: &str = include_str!("../wit/deps/cli/command.wit");
    const DEPS_CLI_ENVIRONMENT_WIT: &str = include_str!("../wit/deps/cli/environment.wit");
    const DEPS_CLI_EXIT_WIT: &str = include_str!("../wit/deps/cli/exit.wit");
    const DEPS_CLI_IMPORTS_WIT: &str = include_str!("../wit/deps/cli/imports.wit");
    const DEPS_CLI_RUN_WIT: &str = include_str!("../wit/deps/cli/run.wit");
    const DEPS_CLI_STDIO_WIT: &str = include_str!("../wit/deps/cli/stdio.wit");
    const DEPS_CLI_TERMINAL_WIT: &str = include_str!("../wit/deps/cli/terminal.wit");
    const DEPS_CLOCKS_MONOTONIC_CLOCK_WIT: &str = include_str!("../wit/deps/clocks/monotonic-clock.wit");
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
    const DEPS_RANDOM_INSECURE_SEED_WIT: &str = include_str!("../wit/deps/random/insecure-seed.wit");
    const DEPS_RANDOM_INSECURE_WIT: &str = include_str!("../wit/deps/random/insecure.wit");
    const DEPS_RANDOM_RANDOM_WIT: &str = include_str!("../wit/deps/random/random.wit");
    const DEPS_RANDOM_WORLD_WIT: &str = include_str!("../wit/deps/random/world.wit");
    const DEPS_SOCKETS_INSTANCE_NETWORK_WIT: &str = include_str!("../wit/deps/sockets/instance-network.wit");
    const DEPS_SOCKETS_IP_NAME_LOOKUP_WIT: &str = include_str!("../wit/deps/sockets/ip-name-lookup.wit");
    const DEPS_SOCKETS_NETWORK_WIT: &str = include_str!("../wit/deps/sockets/network.wit");
    const DEPS_SOCKETS_TCP_CREATE_SOCKET_WIT: &str = include_str!("../wit/deps/sockets/tcp-create-socket.wit");
    const DEPS_SOCKETS_TCP_WIT: &str = include_str!("../wit/deps/sockets/tcp.wit");
    const DEPS_SOCKETS_UDP_CREATE_SOCKET_WIT: &str = include_str!("../wit/deps/sockets/udp-create-socket.wit");
    const DEPS_SOCKETS_UDP_WIT: &str = include_str!("../wit/deps/sockets/udp.wit");
    const DEPS_SOCKETS_WORLD_WIT: &str = include_str!("../wit/deps/sockets/world.wit");
    // outlayer:api/host removed — P1 only uses wasi:http

    fn build_group(files: &[(&str, &str)]) -> Result<wit_parser::UnresolvedPackageGroup, String> {
        let mut map = SourceMap::default();
        for &(path, contents) in files {
            map.push_str(path, contents);
        }
        map.parse()
            .map_err(|(map, e)| format!("WIT parse error: {}", e.highlight(&map)))
    }
    // Load deps in topological order (io has no deps, everything else depends on it)
    let pkg_deps_io = build_group(&[
        ("deps/io/error.wit", DEPS_IO_ERROR_WIT),
        ("deps/io/poll.wit", DEPS_IO_POLL_WIT),
        ("deps/io/streams.wit", DEPS_IO_STREAMS_WIT),
        ("deps/io/world.wit", DEPS_IO_WORLD_WIT),
    ])?;
    resolve.push_group(pkg_deps_io).map_err(|e| format!("push_group deps/io: {:?}", e))?;

    let pkg_deps_clocks = build_group(&[
        ("deps/clocks/monotonic-clock.wit", DEPS_CLOCKS_MONOTONIC_CLOCK_WIT),
        ("deps/clocks/timezone.wit", DEPS_CLOCKS_TIMEZONE_WIT),
        ("deps/clocks/wall-clock.wit", DEPS_CLOCKS_WALL_CLOCK_WIT),
        ("deps/clocks/world.wit", DEPS_CLOCKS_WORLD_WIT),
    ])?;
    resolve.push_group(pkg_deps_clocks).map_err(|e| format!("push_group deps/clocks: {:?}", e))?;

    let pkg_deps_random = build_group(&[
        ("deps/random/insecure-seed.wit", DEPS_RANDOM_INSECURE_SEED_WIT),
        ("deps/random/insecure.wit", DEPS_RANDOM_INSECURE_WIT),
        ("deps/random/random.wit", DEPS_RANDOM_RANDOM_WIT),
        ("deps/random/world.wit", DEPS_RANDOM_WORLD_WIT),
    ])?;
    resolve.push_group(pkg_deps_random).map_err(|e| format!("push_group deps/random: {:?}", e))?;

    let pkg_deps_filesystem = build_group(&[
        ("deps/filesystem/preopens.wit", DEPS_FILESYSTEM_PREOPENS_WIT),
        ("deps/filesystem/types.wit", DEPS_FILESYSTEM_TYPES_WIT),
        ("deps/filesystem/world.wit", DEPS_FILESYSTEM_WORLD_WIT),
    ])?;
    resolve.push_group(pkg_deps_filesystem).map_err(|e| format!("push_group deps/filesystem: {:?}", e))?;

    let pkg_deps_sockets = build_group(&[
        ("deps/sockets/instance-network.wit", DEPS_SOCKETS_INSTANCE_NETWORK_WIT),
        ("deps/sockets/ip-name-lookup.wit", DEPS_SOCKETS_IP_NAME_LOOKUP_WIT),
        ("deps/sockets/network.wit", DEPS_SOCKETS_NETWORK_WIT),
        ("deps/sockets/tcp-create-socket.wit", DEPS_SOCKETS_TCP_CREATE_SOCKET_WIT),
        ("deps/sockets/tcp.wit", DEPS_SOCKETS_TCP_WIT),
        ("deps/sockets/udp-create-socket.wit", DEPS_SOCKETS_UDP_CREATE_SOCKET_WIT),
        ("deps/sockets/udp.wit", DEPS_SOCKETS_UDP_WIT),
        ("deps/sockets/world.wit", DEPS_SOCKETS_WORLD_WIT),
    ])?;
    resolve.push_group(pkg_deps_sockets).map_err(|e| format!("push_group deps/sockets: {:?}", e))?;

    let pkg_deps_cli = build_group(&[
        ("deps/cli/command.wit", DEPS_CLI_COMMAND_WIT),
        ("deps/cli/environment.wit", DEPS_CLI_ENVIRONMENT_WIT),
        ("deps/cli/exit.wit", DEPS_CLI_EXIT_WIT),
        ("deps/cli/imports.wit", DEPS_CLI_IMPORTS_WIT),
        ("deps/cli/run.wit", DEPS_CLI_RUN_WIT),
        ("deps/cli/stdio.wit", DEPS_CLI_STDIO_WIT),
        ("deps/cli/terminal.wit", DEPS_CLI_TERMINAL_WIT),
    ])?;
    resolve.push_group(pkg_deps_cli).map_err(|e| format!("push_group deps/cli: {:?}", e))?;

    let pkg_deps_http = build_group(&[
        ("deps/http/handler.wit", DEPS_HTTP_HANDLER_WIT),
        ("deps/http/proxy.wit", DEPS_HTTP_PROXY_WIT),
        ("deps/http/types.wit", DEPS_HTTP_TYPES_WIT),
    ])?;
    resolve.push_group(pkg_deps_http).map_err(|e| format!("push_group deps/http: {:?}", e))?;

    // outlayer:api/host removed — upstream uses split interfaces, not loaded for P1

    let pkg_main = build_group(&[
        ("world.wit", WORLD_WIT),
    ])?;
    let pkg_id = resolve.push_group(pkg_main).map_err(|e| format!("push_group main: {:?}", e))?;

    let pkg = &resolve.packages[pkg_id];
    let world = pkg.worlds.iter()
        .find_map(|(name, id)| if name == "simple-http" { Some(*id) } else { None })
        .ok_or("world 'simple-http' not found")?;

    Ok((resolve, world))
}