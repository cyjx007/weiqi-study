{
  description = "Local Go study tools: KataGo review + terminal board";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        weiqi-study = pkgs.callPackage ./nix/package.nix { };
        default = weiqi-study;
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.katago
            pkgs.pkg-config
            pkgs.gtk4
            pkgs.libadwaita
            pkgs.gobject-introspection
          ];
        };
      });
    };
}
