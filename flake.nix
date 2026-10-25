{
  description = "protonhub - install and manage Proton, Wine and DXVK builds across game launchers";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    let
      # Packaging lives in packaging/ (packaging/README.md).
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
      protonhub = pkgs: pkgs.callPackage ./packaging/nix/package.nix { inherit version; };
    in
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        formatter = pkgs.nixfmt;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            just
            pkg-config # finds gtk4 and libadwaita for crates/gtk
            gtk4
            libadwaita
            udev # gamepads, through gilrs
            gettext # translations: just pot, just locale
            sway # a private headless display for scripts/screenshot.sh
            grim # screenshots of it
            wtype # types into it
            vulkan-tools # vkcube, which the MangoHud editor previews over
          ];

          env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          # GTK's settings schemas, which the packaged app gets from
          # wrapGAppsHook4; without them `just gui` logs GSettings criticals.
          shellHook = ''
            export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
          '';
        };

        packages.default = protonhub pkgs;
      }
    )
    // {
      overlays.default = final: _: { protonhub = protonhub final; };
      nixosModules.default = import ./packaging/nix/nixos-module.nix self;
      homeManagerModules.default = import ./packaging/nix/hm-module.nix self;
    };
}
