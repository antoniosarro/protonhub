{
  lib,
  rustPlatform,
  pkg-config,
  wrapGAppsHook4,
  gettext,
  gtk4,
  libadwaita,
  udev,
  vulkan-tools,
  version ? "0.0.0-dev",
}:

rustPlatform.buildRustPackage {
  pname = "protonhub";
  inherit version;

  # Only what the build reads, so editing docs or packaging doesn't rebuild.
  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../Cargo.toml
      ../../Cargo.lock
      ../../crates
      ../../registry # installed, and the registry tests read it
      ../../po
      ../../packaging/linux
      ../../packaging/install-data.sh
    ];
  };

  cargoLock.lockFile = ../../Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook4
    gettext # msgfmt, for the translations
  ];
  buildInputs = [
    gtk4
    libadwaita
    udev
  ];

  # vkcube, which the MangoHud editor previews over; MangoHud itself comes
  # from the system, as its Vulkan layer must.
  preFixup = ''
    gappsWrapperArgs+=(--suffix PATH : ${lib.makeBinPath [ vulkan-tools ]})
  '';

  postInstall = ''
    sh packaging/install-data.sh $out
  '';

  # Every crate, not just the default members, so the GUI is built too.
  cargoBuildFlags = [ "--workspace" ];
  cargoTestFlags = [ "--workspace" ];

  meta = {
    description = "Install and manage Proton, Wine and DXVK builds across game launchers";
    license = lib.licenses.mit;
    homepage = "https://github.com/antoniosarro/protonhub";
    mainProgram = "protonhub";
    platforms = lib.platforms.linux;
  };
}
