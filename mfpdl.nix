{ sources ? import ./nix/sources.nix, pkgs ? import sources.nixpkgs {} }:
let
  rust = import ./nix/rust.nix { inherit sources; };

  naersk = pkgs.callPackage sources.naersk {
    rustc = rust;
    cargo = rust;
  };

  src = builtins.filterSource
  (path: type: type != "directory" || builtins.baseNameOf path != "target")
  ./.;
  deps = import ./common_deps.nix;
in naersk.buildPackage {
  inherit src;
  buildInputs = deps {inherit pkgs;};
  remapPathPrefix = true;
}
