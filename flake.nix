{
  description = "t3boat: auto-sleeping T3 Code machines on Boat, with a headless server and web UI";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      # The desktop (Tauri) build needs webkit and the tray libraries; the server build does not.
      desktopLibs = with pkgs; [
        webkitgtk_4_1 libsoup_3 gtk3 glib cairo pango gdk-pixbuf atk librsvg libayatana-appindicator openssl xdotool
      ];
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc clippy rustfmt pkg-config bun nodejs_22 playwright-driver.browsers ] ++ desktopLibs;
        PLAYWRIGHT_BROWSERS_PATH = "${pkgs.playwright-driver.browsers}";
        PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "true";
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ];
      };
    };
}
