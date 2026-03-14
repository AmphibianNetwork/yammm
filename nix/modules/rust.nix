{ inputs, ... }:
{
  imports = [
    inputs.rust-flake.flakeModules.default
    inputs.rust-flake.flakeModules.nixpkgs
  ];
  perSystem =
    {
      config,
      self',
      pkgs,
      lib,
      ...
    }:
    {
      rust-project.crates."yammm".path = lib.mkDefault config.rust-project.src;
      rust-project.crates."yammm".crane.args = {
        buildInputs = lib.optionals pkgs.stdenv.isLinux [
          pkgs.libx11
          pkgs.libxcursor
          pkgs.libxi
          pkgs.libxrandr
          pkgs.wayland
          pkgs.libxkbcommon
          pkgs.openssl
        ];
        nativeBuildInputs = lib.optionals pkgs.stdenv.isLinux [
          pkgs.pkg-config
        ];
      };
      packages.default = self'.packages.yammm;
    };
}
