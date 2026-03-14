{ inputs, ... }:
{
  imports = [ (inputs.git-hooks + /flake-module.nix) ];
  perSystem =
    {
      config,
      self',
      pkgs,
      lib,
      ...
    }:
    {
      pre-commit.settings = {
        hooks = {
          nixfmt.enable = true;
          rustfmt.enable = true;
        };
      };
    };
}
