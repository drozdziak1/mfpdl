let
  sources = import ./nix/sources.nix;
  rust = import ./nix/rust.nix { inherit sources; };
  pkgs = import sources.nixpkgs {};
  deps = import ./common_deps.nix;
in
pkgs.mkShell {
  buildInputs = [
  rust
  ] ++ deps {inherit pkgs;};
}
