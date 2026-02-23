+++
title = "Nix Flakes Cheatsheet"
tags = ["nix", "devops", "reproducibility"]
date = 2025-08-14
draft = false
+++

# Nix Flakes Cheatsheet

Flakes are the standard way to define Nix projects with hermetic, reproducible dependencies.

## Basic Commands

- `nix flake init` -- scaffold a new flake in the current directory
- `nix flake update` -- update all inputs in `flake.lock`
- `nix flake show` -- list the outputs of a flake
- `nix develop` -- enter the dev shell defined by the flake

## Minimal flake.nix

```nix
{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "aarch64-darwin";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      devShells.${system}.default = pkgs.mkShell {
        packages = [ pkgs.rustc pkgs.cargo pkgs.rust-analyzer ];
      };
    };
}
```

## Gotchas

Flakes require all sources to be tracked by git. Untracked files are invisible to the build. Run `git add .` before `nix build` if you get mysterious missing-file errors.
