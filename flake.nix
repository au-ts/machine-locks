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
          openssl
          pkg-config
        ];
      };
    }) inputs.nixpkgs.legacyPackages;

    packages = builtins.mapAttrs (system: pkgs: {
      server = pkgs.rustPlatform.buildRustPackage {
        pname = "machine-locks-server";
        src = ./.;
        version = "0.1.0";
        nativeBuildInputs = [
          pkgs.pkg-config
        ];
        buildInputs = [
          pkgs.sqlite
        ];
        doCheck = false;
        buildAndTestSubdir = "machine_locks_server";
        cargoLock = {
          lockFile = ./Cargo.lock;
        };
      };

      client = pkgs.rustPlatform.buildRustPackage {
        pname = "machine-locks-client";
        src = ./.;
        version = "0.1.0";
        buildAndTestSubdir = "machine_locks_client";
        nativeBuildInputs = [
          pkgs.pkg-config
        ];
        doCheck = false;
        buildInputs = [
          pkgs.aws-lc
          pkgs.openssl
        ];
        cargoLock = {
          lockFile = ./Cargo.lock;
        };
      };
    }) inputs.nixpkgs.legacyPackages;
  };
}
