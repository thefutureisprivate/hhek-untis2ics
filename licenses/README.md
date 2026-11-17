# Third-party notices

The container build collects license, copyright and notice files from the
Cargo registry sources used by the build, including bundled native libraries.
It also includes the Rust toolchain's standard-library copyright report and
license texts. These files retain their original paths under `crates/` and
`rust/` in `/usr/share/licenses/hhek-untis2ics`.

`musl-COPYRIGHT` is the unmodified copyright notice from musl 1.2.6, obtained
from Alpine's official source archive:

<https://distfiles.alpinelinux.org/distfiles/v3.24/musl-1.2.6.tar.gz>

The archive was verified against the SHA-512 in Alpine's musl APKBUILD:

```text
1adad96eddb3a2eb0cacb3e363b0046568925fcdd75cf8b0503f2139df1f693d64730779ca0ce8131b7624ab2d37f4247bb1d3393c523de6e30d2b1d7732555c
```

Review this notice when updating the Rust/Alpine builder's musl version.
