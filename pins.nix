# Inherit pinning from flake.lock.
let
  lock = builtins.fromJSON (builtins.readFile ./flake.lock);
  fetchGithub =
    name:
    let
      n = lock.nodes.${name}.locked;
    in
    builtins.fetchTree {
      inherit (n)
        type
        owner
        repo
        narHash
        rev
        ;
    };
in
{
  nixpkgs = fetchGithub "nixpkgs";
  rust-overlay = fetchGithub "rust-overlay";
  crane = fetchGithub "crane";
}
