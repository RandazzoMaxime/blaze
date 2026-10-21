{
  description = "ra-jit: a compiler daemon and code generator, with live edits";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ] (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        # The one toolchain ra-jit is pinned to (harness/toolchain); rust-src is the sysroot
        # source rust-analyzer reads `core` from.
        rust = pkgs.rust-bin.stable."1.99.0".default.override {
          extensions = [ "rust-src" ];
        };
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rust;
          rustc = rust;
        };
        src = self;
        # The one version, from the workspace's Cargo.toml.
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
        # The one build of ra-jit, the release workflow's recipe (.github/scripts/build_release.py):
        # the `dist` profile optimized with the profile pgo/profiles.json pins for this system,
        # fetched by its sha256. The binaries run each other from next to themselves, unwrapped.
        triple = {
          x86_64-linux = "x86_64-unknown-linux-gnu";
          aarch64-linux = "aarch64-unknown-linux-gnu";
          aarch64-darwin = "aarch64-apple-darwin";
        }.${system};
        pgo = builtins.fromJSON (builtins.readFile ./pgo/profiles.json);
        pgoProfile =
          let pin = pgo.targets.${triple} or (throw "pgo/profiles.json has no profile for ${triple}: run the pgo workflow");
          in pkgs.fetchurl { url = "${pgo.release}/${pin.name}"; inherit (pin) sha256; };
        dist = rustPlatform.buildRustPackage {
          pname = "ra-jit-dist";
          inherit version src;
          # `ra-jit --version` names the commit (a dirty tree has none of its own).
          RA_JIT_GIT_SHA = self.shortRev or self.dirtyShortRev or "";
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.python3 ];
          doCheck = false;
          # cargo-auditable wraps rustc for the workspace's crates, which changes their symbols:
          # the profile would find none of them.
          auditable = false;
          buildPhase = ''
            runHook preBuild
            python3 -I .github/scripts/build_release.py --target ${triple} --profdata ${pgoProfile} --out shipped
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p $out/bin
            cp shipped/* $out/bin/
            runHook postInstall
          '';
        };
        # What `nix profile install` puts on PATH: the same binaries, with the toolchain.
        daemon = pkgs.runCommand "ra-jit-${version}" {
          nativeBuildInputs = [ pkgs.makeWrapper ];
          meta.mainProgram = "cargo-ra-jit";
        } ''
          mkdir -p $out/bin
          for bin in ${dist}/bin/*; do
            makeWrapper $bin $out/bin/$(basename $bin) --prefix PATH : ${rust}/bin
          done
        '';
        # `nix run github:indexable-inc/ra-jit -- build` is `cargo ra-jit build`.
        run = pkgs.writeShellScriptBin "ra-jit" ''
          export PATH=${daemon}/bin:${rust}/bin:$PATH
          exec ${daemon}/bin/cargo-ra-jit ra-jit "$@"
        '';

        # eframe/winit dlopen these at run time.
        guiLibs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (with pkgs; [
          libGL
          libxkbcommon
          wayland
          libx11
          libxcursor
          libxi
          libxrandr
          libxcb
        ]);
        libPath = pkgs.lib.makeLibraryPath guiLibs;

        # One derivation per cargo unit (nix/units.nix, nix/units/*.json from nix/unit_graph.py):
        # editing ra-jit's own source rebuilds only ra-jit, never the crates it depends on. The
        # demo's window is built this way; `daemon-units` is ra-jit built this way, `release`
        # without PGO, for working on the flake: what ships is `dist`.
        units = json: import ./nix/units.nix {
          inherit pkgs rust guiLibs json;
          repo = ./.;
          gitSha = self.shortRev or self.dirtyShortRev or "";
        };
        daemonUnits = units ./nix/units/daemon.json;
        lifeUnits = units ./nix/units/life.json;
        # ra-jit's binaries run each other from next to themselves, found by
        # their own path: the binaries are copied together, not wrapped or linked. The toolchain
        # has to be on PATH (the launcher puts it there).
        daemon-units = pkgs.runCommand "ra-jit-units" { } ''
          mkdir -p $out/bin
          cp -L ${daemonUnits.bins}/bin/* $out/bin/
        '';
        life-bin = pkgs.runCommand "ra-jit-life-bin" { nativeBuildInputs = [ pkgs.makeWrapper ]; } ''
          mkdir -p $out/bin
          makeWrapper ${builtins.head lifeUnits.roots}/bin/life $out/bin/life \
            --prefix LD_LIBRARY_PATH : ${libPath}
        '';

        # The demo's own source (and the crate it links), copied to a writable directory at run
        # time; its crates.io dependencies are vendored from Cargo.lock so that needs no network.
        lifeSource = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [ ./demo/life ./crates/ra-jit-live ];
        };
        lifeVendor = pkgs.rustPlatform.importCargoLock { lockFile = ./demo/life/Cargo.lock; };
        # `nix run github:indexable-inc/ra-jit`: the Game of Life window. On macOS on AArch64 the
        # daemon builds the copy and edits to it show up live; elsewhere the prebuilt window runs.
        life = pkgs.writeShellScriptBin "ra-jit-life" ''
          export RA_JIT_LIFE_SOURCE=${lifeSource} RA_JIT_LIFE_VENDOR=${lifeVendor}
          export RA_JIT_LIFE_DAEMON_BINS=${dist}/bin RA_JIT_LIFE_PREBUILT=${life-bin}/bin/life
          export RA_JIT_LIFE_PATH=${rust}/bin${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ":${pkgs.stdenv.cc}/bin"}
          export RA_JIT_LIFE_LIBS=${libPath}
          exec ${pkgs.python3}/bin/python3 -I ${./nix/life_launch.py} "$@"
        '';
      in {
        packages = {
          default = life;
          # `nix profile install github:indexable-inc/ra-jit#ra-jit`: ra-jit, cargo-ra-jit and
          # ra-jit-rustc, so `ra-jit enable` and `cargo ra-jit` both work.
          ra-jit = daemon;
          inherit dist daemon run life daemon-units life-bin;
        };
        apps.default = {
          type = "app";
          program = "${life}/bin/ra-jit-life";
        };
        apps.ra-jit = {
          type = "app";
          program = "${run}/bin/ra-jit";
        };
        devShells.default = pkgs.mkShell {
          packages = [ rust pkgs.python3 ];
        };
        # What harness/bench.py runs in on the benchmark machine (docs/benchmarks.md): the
        # pinned toolchain, LLVM 20 for `--features llvm`, and the C side the projects' build
        # scripts use. Nothing here wraps rustc (no sccache) or replaces the linker (no mold).
        devShells.bench = pkgs.mkShell {
          packages = [
            rust
            pkgs.python3
            pkgs.git
            pkgs.pkg-config
            pkgs.openssl
            pkgs.llvmPackages_20.llvm.dev
            pkgs.libffi
            pkgs.libxml2
            pkgs.zlib
            pkgs.ncurses
          ];
          LLVM_SYS_201_PREFIX = "${pkgs.llvmPackages_20.llvm.dev}";
          # A C compiler as a distribution ships it: the wrapper's _FORTIFY_SOURCE warns at the
          # -O0 of a debug build, which fails configure probes (jemalloc's, in fd).
          hardeningDisable = [ "all" ];
        };
      });
}
