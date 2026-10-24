# home-manager: installs protonhub, and can declare its channels (the tools
# kept at their latest release, docs/usage.md#channels) and the daily timer
# that updates them.
self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.protonhub;
  toml = pkgs.formats.toml { };

  channel = lib.types.submodule {
    options = {
      tool = lib.mkOption {
        type = lib.types.str;
        example = "ge-proton";
        description = "The tool's id in the registry (`protonhub tools`).";
      };
      launcher = lib.mkOption {
        type = lib.types.str;
        example = "steam";
        description = "The launcher: a kind (steam, heroic, lutris, bottles, winezgui), or kind:packaging (steam:flatpak).";
      };
      keep = lib.mkOption {
        type = lib.types.ints.positive;
        default = 3;
        description = "How many versions to keep, newest first; older ones go once no game uses them.";
      };
      moveGames = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Move games set to an older version to the new one.";
      };
    };
  };
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

    channels = lib.mkOption {
      type = lib.types.nullOr (lib.types.listOf channel);
      default = null;
      example = lib.literalExpression ''[ { tool = "ge-proton"; launcher = "steam"; keep = 2; moveGames = true; } ]'';
      description = ''
        The channels, written to ~/.config/protonhub/channels.toml. Null
        leaves that file to `protonhub channel` and the GUI; once set here,
        the file is read-only and channels change only through this option.
      '';
    };

    timer = {
      enable = lib.mkEnableOption "a systemd user timer that runs `protonhub update --notify`";
      onCalendar = lib.mkOption {
        type = lib.types.str;
        default = "daily";
        description = "When to update, as systemd.time(7) calendar events.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."protonhub/channels.toml" = lib.mkIf (cfg.channels != null) {
      source = toml.generate "channels.toml" {
        channel = map (c: {
          inherit (c) tool launcher keep;
          move_games = c.moveGames;
        }) cfg.channels;
      };
    };

    # What `protonhub timer enable` writes, declared here instead.
    systemd.user.services.protonhub-update = lib.mkIf cfg.timer.enable {
      Unit.Description = "Update ProtonHub's release channels";
      Service = {
        Type = "oneshot";
        ExecStart = "${lib.getExe cfg.package} update --notify";
      };
    };
    systemd.user.timers.protonhub-update = lib.mkIf cfg.timer.enable {
      Unit.Description = "Update ProtonHub's release channels";
      Timer = {
        OnCalendar = cfg.timer.onCalendar;
        Persistent = true;
        RandomizedDelaySec = "1h";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
