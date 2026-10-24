# NixOS: installs protonhub (the CLI and the GUI) for every user.
self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.protonhub;
in
{
  options.programs.protonhub = {
    enable = lib.mkEnableOption "protonhub, a manager of Proton, Wine, DXVK and VKD3D builds";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "protonhub.packages.\${system}.default";
      description = "The protonhub package to install.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
  };
}
