{
  pkgs,
  craneLib,
}:
let
  inherit (pkgs) lib;
  # Hub / agent / TURN: fully static musl. Tauri cannot (WebKit).
  muslTarget =
    if pkgs.stdenv.hostPlatform.isLinux then
      "${pkgs.stdenv.hostPlatform.parsed.cpu.name}-unknown-linux-musl"
    else
      null;
  rustToolchain =
    let
      base = pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
    in
    if muslTarget == null then
      base
    else
      base.override {
        targets = [
          "wasm32-unknown-unknown"
          muslTarget
        ];
      };
  craneLib' = craneLib.overrideToolchain rustToolchain;

  staticCli =
    if muslTarget == null then
      { }
    else
      let
        envKey =
          suffix: "CARGO_TARGET_${lib.toUpper (lib.replaceStrings [ "-" ] [ "_" ] muslTarget)}_${suffix}";
      in
      {
        CARGO_BUILD_TARGET = muslTarget;
        TARGET_CC = "${pkgs.pkgsStatic.stdenv.cc}/bin/${pkgs.pkgsStatic.stdenv.cc.targetPrefix}cc";
        "CC_${lib.replaceStrings [ "-" ] [ "_" ] muslTarget}" =
          "${pkgs.pkgsStatic.stdenv.cc}/bin/${pkgs.pkgsStatic.stdenv.cc.targetPrefix}cc";
        ${envKey "LINKER"} = "${pkgs.pkgsStatic.stdenv.cc}/bin/${pkgs.pkgsStatic.stdenv.cc.targetPrefix}cc";
        ${envKey "RUSTFLAGS"} = "-C target-feature=+crt-static";
      };

  srcFilter =
    path: type:
    let
      base = baseNameOf path;
    in
    !(base == "target" || lib.hasInfix "/target/" path)
    && !(base == ".sysroot" || lib.hasInfix "/.sysroot/" path)
    && (
      (craneLib'.filterCargoSources path type)
      || lib.hasInfix "/vendor/" path
      || lib.hasInfix "/crates/" path
      || lib.hasInfix "/ui/" path
      || lib.hasInfix "/web/" path
      || lib.hasInfix "/src-tauri" path
      || base == "src-tauri"
      || base == "style.css"
      || lib.hasSuffix ".css" path
      || lib.hasSuffix ".js" path
      || lib.hasSuffix ".html" path
    );

  src = lib.cleanSourceWith {
    src = ../.;
    filter = srcFilter;
    name = "connect2";
  };

  cargoVendorDir = craneLib'.vendorMultipleCargoDeps {
    cargoLockList = [
      ../Cargo.lock
      ../ui/Cargo.lock
    ];
  };

  commonArgs = {
    inherit src cargoVendorDir;
    pname = "connect2-agent";
    version = "0.1.0";
    strictDeps = true;
    doCheck = false;
    nativeBuildInputs = [
      rustToolchain
      pkgs.pkg-config
    ]
    ++ lib.optionals (muslTarget != null) [ pkgs.pkgsStatic.stdenv.cc ];
    CONNECT2_SKIP_CSS = "1";
    CONNECT2_KEEP_RUSTC = "1";
    preBuild = ''
      export CONNECT2_UI_TARGET_DIR="$PWD/ui-target"
      mkdir -p "$CONNECT2_UI_TARGET_DIR"
    '';
    CARGO_BUILD_INCREMENTAL = "false";
  }
  // staticCli;

  cargoArtifacts = craneLib'.buildDepsOnly (
    commonArgs
    // {
      cargoExtraArgs = "--locked --offline --bin connect2 --bin connect2-agent --bin connect2-turn --features web";
    }
  );

  client = craneLib'.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      pname = "connect2";
      cargoExtraArgs = "--locked --offline --bin connect2 --features web";
      postInstall = ''
        mkdir -p $out/share/connect2/ui
        if [ ! -f ui/public/index.html ]; then
          echo "ui/public missing after web build" >&2
          exit 1
        fi
        cp -a ui/public/. $out/share/connect2/ui/
      '';
    }
  );

  agent = craneLib'.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      pname = "connect2-agent";
      cargoExtraArgs = "--locked --offline --bin connect2-agent --no-default-features";
    }
  );

  turn = craneLib'.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      pname = "connect2-turn";
      cargoExtraArgs = "--locked --offline --bin connect2-turn --no-default-features";
    }
  );

  tauriLinux =
    let
      tauriVendorDir = craneLib'.vendorMultipleCargoDeps {
        cargoLockList = [
          ../Cargo.lock
          ../src-tauri/Cargo.lock
          ../ui/Cargo.lock
        ];
      };
      tauriArgs = {
        inherit src;
        cargoToml = ../src-tauri/Cargo.toml;
        cargoLock = ../src-tauri/Cargo.lock;
        cargoVendorDir = tauriVendorDir;
        pname = "connect2-tauri";
        version = "0.1.0";
        strictDeps = true;
        doCheck = false;
        nativeBuildInputs = [
          rustToolchain
          pkgs.pkg-config
          pkgs.wrapGAppsHook3
        ];
        buildInputs = [
          pkgs.gtk3
          pkgs.glib
          pkgs.cairo
          pkgs.pango
          pkgs.gdk-pixbuf
          pkgs.webkitgtk_4_1
          pkgs.libsoup_3
          pkgs.openssl
        ];
        cargoExtraArgs = "--locked --offline --manifest-path src-tauri/Cargo.toml";
        CARGO_BUILD_INCREMENTAL = "false";
        CONNECT2_SKIP_CSS = "1";
        CONNECT2_KEEP_RUSTC = "1";
        preBuild = ''
          export CONNECT2_UI_TARGET_DIR="$PWD/ui-target"
          mkdir -p "$CONNECT2_UI_TARGET_DIR" ui/public
          cp -a ${client}/share/connect2/ui/. ui/public/
        '';
      };
      tauriArtifacts = craneLib'.buildDepsOnly (
        tauriArgs
        // {
          # lockfile vendor only; UI copy is for the real crate build
          preBuild = "true";
        }
      );
    in
    craneLib'.buildPackage (
      tauriArgs
      // {
        cargoArtifacts = tauriArtifacts;
      }
    );

  tauri = if pkgs.stdenv.isLinux then tauriLinux else null;

  desktop =
    if pkgs.stdenv.isLinux then
      pkgs.symlinkJoin {
        name = "connect2-desktop";
        paths = [
          client
          tauriLinux
        ];
      }
    else
      client;
in
{
  inherit
    client
    agent
    turn
    cargoArtifacts
    rustToolchain
    tauri
    desktop
    ;
  default = client;
}
