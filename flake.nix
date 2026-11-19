{
  description = "t3boat: auto-sleeping T3 Code machines on Boat, with a headless server and web UI";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      lib = pkgs.lib;
      # The desktop (Tauri) build needs webkit and the tray libraries; the server build does not.
      desktopLibs = with pkgs; [
        webkitgtk_4_1 libsoup_3 gtk3 glib cairo pango gdk-pixbuf atk librsvg libayatana-appindicator openssl xdotool
      ];
      src = lib.cleanSourceWith {
        src = ./.;
        filter = path: type:
          let rel = lib.removePrefix (toString ./. + "/") (toString path);
          in !(lib.hasInfix "node_modules" rel || lib.hasInfix "/target" rel || lib.hasInfix "e2e-results" rel
            || lib.hasSuffix "/dist" rel || rel == "apps/web");
      };

      # Bun's install, fetched once by content hash (update `outputHash` when bun.lock changes).
      nodeModules = pkgs.stdenvNoCC.mkDerivation {
        pname = "t3boat-node-modules";
        version = "0";
        inherit src;
        nativeBuildInputs = [ pkgs.bun ];
        dontConfigure = true;
        buildPhase = ''
          export HOME=$TMPDIR
          bun install --frozen-lockfile --no-progress --ignore-scripts --filter @t3boat/desktop
        '';
        installPhase = ''
          mkdir -p $out
          cp -r node_modules $out/root
          [ -d apps/desktop/node_modules ] && cp -r apps/desktop/node_modules $out/desktop || mkdir $out/desktop
        '';
        dontFixup = true;
        outputHashMode = "recursive";
        outputHashAlgo = "sha256";
        outputHash = "sha256-Hc/2ftpnDOorW1ehCgnouNE/NDETBJ7TfRoNnm5M0w8=";
      };

      # The UI as t3boat-server serves it.
      web = pkgs.stdenvNoCC.mkDerivation {
        pname = "t3boat-web";
        version = "0.1.0";
        inherit src;
        nativeBuildInputs = [ pkgs.bun pkgs.nodejs_22 ];
        buildPhase = ''
          export HOME=$TMPDIR
          cp -r ${nodeModules}/root node_modules
          cp -r ${nodeModules}/desktop apps/desktop/node_modules
          chmod -R u+w node_modules apps/desktop/node_modules
          patchShebangs node_modules apps/desktop/node_modules
          cd apps/desktop
          bun run build
        '';
        installPhase = "cp -r dist $out";
      };

      server = pkgs.rustPlatform.buildRustPackage {
        pname = "t3boat-server";
        version = "0.1.0";
        inherit src;
        sourceRoot = "${src.name}/apps/desktop/src-tauri";
        cargoLock.lockFile = ./apps/desktop/src-tauri/Cargo.lock;
        buildNoDefaultFeatures = true;
        cargoBuildFlags = [ "--bin" "t3boat-server" ];
        # The unit tests bind loopback ports and run fake CLIs: run them with `cargo test` in the dev shell.
        doCheck = false;
        postInstall = ''
          mkdir -p $out/share/t3boat
          cp -r ${web} $out/share/t3boat/web
        '';
        meta.mainProgram = "t3boat-server";
      };
    in
    {
      packages.${system} = {
        t3boat-server = server;
        t3boat-web = web;
        default = server;
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc clippy rustfmt pkg-config bun nodejs_22 playwright-driver.browsers ] ++ desktopLibs;
        PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
        PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "true";
        LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.libayatana-appindicator ];
      };
    };
}
