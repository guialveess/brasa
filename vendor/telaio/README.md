# Vendored telaio

A trimmed, in-tree copy of [telaio](https://github.com/tanisperez/telaio) (`telaio-ui`,
`telaio-app`, `telaio-git`, `telaio-update`, checked out at v0.15.0), restyled for Brasa: the
border and spacing fixes, the ember palette tokens and the bumped metrics this repo's own history
describes are already in here.

It lives in the repository (not as a sibling checkout or a separate fork) so a plain `git clone`
of Brasa builds on its own, in CI included: the root `Cargo.toml`'s
`[patch."https://github.com/tanisperez/telaio"]` block points `telaio-ui`, `telaio-app`,
`telaio-git` and `telaio-update` at `vendor/telaio/crates/*`. `telaio-gallery`, telaio's own demo
app, is not needed here and was left out.

To update to a newer telaio release, replace these 4 crate directories (and this `Cargo.toml`'s
dependency versions) with the new tag's, then reapply Brasa's own changes on top.
