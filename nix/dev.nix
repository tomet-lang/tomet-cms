{
  pkgs,
  mkShell,
  fenix,
  tomet,
  twrit,
  catasto,
  ...
}:
let
  rustToolchain = fenix.combine [
    (fenix.stable.withComponents [
      "cargo"
      "clippy"
      "rustc"
      "rust-src"
      "rustfmt"
      "rust-analyzer"
    ])
  ];
in
mkShell {
  buildInputs = with pkgs; [
    tomet
    twrit
    catasto
    just

    #= Build
    pkg-config

    #= Rust
    rustToolchain
    cargo-edit
    cargo-outdated
    cargo-nextest
  ];

  shellHook = ''
    echo "🦀📚 Rust Tomet (tomet-cms)"
  '';
}
