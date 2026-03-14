{ self, ... }:
{
  flake = {
    overlays.default = final: prev: {
      yammm = self.packages.${prev.system}.default;
    };
  };
}
