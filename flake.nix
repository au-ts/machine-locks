{
  description = "Server for locking mq.sh and iq.sh.";

  inputs = {
    nixpkgs.url = "https://channels.nixos.org/nixpkgs-unstable/nixexprs.tar.zst";
  };

  outputs = inputs: {
    devShells = builtins.mapAttrs (system: pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [
          rustc
          cargo
          rustfmt
          clippy
          rust-analyzer
          sqlx-cli
        ];
      };
    }) inputs.nixpkgs.legacyPackages;

    packages = builtins.mapAttrs (system: pkgs: {
      server = pkgs.rustPlatform.buildRustPackage {
        pname = "machine_locks_server";
        src = ./.;
        version = "0.1.0";
        nativeBuildInputs = [
          pkgs.pkg-config
        ];
        buildInputs = [ pkgs.sqlite ];
        cargoFlags = [
          "-p"
          "machine_locks_server"
        ];
        cargoLock = {
          lockFile = ./Cargo.lock;
        };
      };
    }) inputs.nixpkgs.legacyPackages;
  };
}
