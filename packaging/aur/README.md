# AUR packaging

Two packages, one binary:

| Package    | What it does                                                              |
| ---------- | ------------------------------------------------------------------------- |
| `ivar-bin` | Installs the prebuilt `ivar-linux-<arch>` asset from the GitHub Release.   |
| `ivar`     | Builds from the source tag with the local Rust toolchain.                  |

They conflict with each other and `ivar-bin` provides `ivar`, so either one
satisfies a dependency on `ivar`.

```sh
yay -S ivar-bin    # no Rust toolchain, seconds
yay -S ivar        # builds from source
```

## These files are the source of truth

The AUR requires each package to live in its own git repo with a generated
`.SRCINFO`. Those repos are **publish targets**, not places to edit:
`.github/workflows/release-aur.yml` copies the `PKGBUILD` from here, rewrites
`pkgver` / `pkgrel` / `sha256sums`, regenerates `.SRCINFO`, and pushes. An edit
made directly in an AUR checkout is silently reverted by the next release.

Change the packaging here, and let a release carry it.

## What ships automatically

`release-plz.yml` runs `release-aur.yml` after `release-binaries.yml`, on the
tag release-plz created. It is gated on `AUR_SSH_PRIVATE_KEY` being present in
repo Settings → Secrets and variables → Actions — an SSH key registered on an
AUR account with maintainer access to both `ivar` and `ivar-bin`. Without it
the job emits a warning and passes green, so a fork never gets a red release.

The ordering is not cosmetic: `ivar-bin`'s sources *are* the release assets, and
it reads their checksums out of the `.sha256` sidecars `release-binaries.yml`
uploaded — the same bytes `scripts/install.sh` verifies against. Publishing the
AUR package before those assets exist would publish checksums for nothing.

Before pushing, `makepkg --verifysource` downloads every source the PKGBUILD
names and checks it against the rendered sums. On the x86_64 runner that covers
the tag tarball for `ivar`, and the licence texts plus the x86_64 binary for
`ivar-bin`. The aarch64 sum is not reachable from that host; it is taken from
the sidecar published beside the asset itself.

## Rerunning by hand

If the AUR push fails after the release is already out — a rotated key, an AUR
outage — re-run just that step. Nothing else needs to be redone:

```sh
gh workflow run release-aur.yml -f tag=v0.1.0
```

The job is idempotent: if the AUR repo already carries that version, it prints
`already at <version>, nothing to publish` and exits clean.

## Publishing without CI

Only needed if the workflow itself is broken.

```sh
version=0.1.0
tag="v${version}"
raw="https://raw.githubusercontent.com/mnzsss/ivar/${tag}"
rel="https://github.com/mnzsss/ivar/releases/download/${tag}"

# Source package
curl -fsSL "https://github.com/mnzsss/ivar/archive/refs/tags/${tag}.tar.gz" | sha256sum

# Binary package
curl -fsSL "$raw/LICENSE" | sha256sum
curl -fsSL "$raw/NOTICE"  | sha256sum
curl -fsSL "$rel/ivar-linux-x86_64.sha256"   # already a sha256; take field 1
curl -fsSL "$rel/ivar-linux-aarch64.sha256"
```

Then, in each AUR checkout: copy the `PKGBUILD` from here, set `pkgver` and
`pkgrel=1`, paste the sums, and

```sh
makepkg --verifysource
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO && git commit -m "Update to ${version}" && git push
```

Every `sha256sums` entry in this directory is a literal `SKIP` on purpose — it
is a placeholder the release workflow overwrites, and the workflow fails loudly
if a `SKIP` survives rendering. Never push a `SKIP` to the AUR.

## `pkgrel`

The workflow always writes `pkgrel=1`, because every run of it is a new
upstream version. A `pkgrel` bump means "same upstream version, repackaged" —
a dependency rebuild, a fixed `install -Dm` mode — and that is a manual push to
the AUR repo by definition. Bump it there and mirror the packaging change back
into this directory, or the next release resets it.

## Testing a change locally

Requires an Arch box or container, and a released tag to point at. From this
directory:

```sh
tag=v0.1.0
rel="https://github.com/mnzsss/ivar/releases/download/${tag}"
raw="https://raw.githubusercontent.com/mnzsss/ivar/${tag}"

mkdir -p /tmp/ivar-aur && cp PKGBUILD-bin /tmp/ivar-aur/PKGBUILD && cd /tmp/ivar-aur
sed -i \
  -e "s/^pkgver=.*/pkgver=${tag#v}/" \
  -e "s/^sha256sums=(.*)/sha256sums=('$(curl -fsSL "$raw/LICENSE" | sha256sum | cut -d' ' -f1)' '$(curl -fsSL "$raw/NOTICE" | sha256sum | cut -d' ' -f1)')/" \
  -e "s/^sha256sums_x86_64=(.*)/sha256sums_x86_64=('$(curl -fsSL "$rel/ivar-linux-x86_64.sha256" | cut -d' ' -f1)')/" \
  PKGBUILD

makepkg --verifysource         # a SKIP that survived would pass here silently
makepkg -Ccf --noconfirm
namcap ivar-bin-*.pkg.tar.zst  # optional lint, `pacman -S namcap`
```

Leave `sha256sums_aarch64` alone unless you are on aarch64 — `--verifysource`
only fetches the sources for the host architecture, so a wrong sum there is not
something this can catch.

`check()` in the source `PKGBUILD` runs the full test suite. It sets `HOME` and
a git identity first, and that is load-bearing: `ivar`'s own commit paths
deliberately inherit the machine's git config rather than forcing an identity,
so a build chroot without one fails the suite on `empty ident name`.
