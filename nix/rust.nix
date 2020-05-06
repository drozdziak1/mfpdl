{ sources ? import ./sources.nix }:

let
  pkgs =
  import sources.nixpkgs {overlays = [ (import sources.nixpkgs-mozilla) ]; };
  channel = "nightly";
  date = null;
  targets = [];
  chan = pkgs.rustChannelOfTargets channel date targets;
in chan
