{
  description = "instantmenu - dynamic menu for X11 and Wayland";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      mkInstantmenu =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "instantmenu";
          version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [
            pkgs.libxkbcommon
            pkgs.libxcb
            pkgs.fontconfig
          ];

          cargoBuildFlags = [
            "--bin"
            "instantmenu"
            "--bin"
            "itest"
          ];

          doCheck = false;

          postInstall = ''
            install -Dm644 instantmenu.1 $out/share/man/man1/instantmenu.1
            install -Dm755 instantmenu_path instantmenu_run -t $out/bin
          '';

          meta = {
            description = "Dynamic menu for X11 and Wayland";
            homepage = "https://github.com/instantOS/instantMENU";
            license = pkgs.lib.licenses.mit;
            mainProgram = "instantmenu";
          };
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = mkInstantmenu pkgs;
          instantmenu = self.packages.${system}.default;
        }
      );

      overlays.default = final: _prev: {
        instantmenu = mkInstantmenu final;
      };

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
            packages = [
              pkgs.cargo
              pkgs.rustc
              pkgs.rust-analyzer
              pkgs.just
              pkgs.python3
            ];
          };
        }
      );

      checks = forAllSystems (system: {
        inherit (self.packages.${system}) default;
      });
    };
}
