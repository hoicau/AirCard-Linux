# AirCard-Linux v0.1.2

- Bundle matching libimobiledevice, libusbmuxd and libplist libraries (and native glue
  when required) in release archives. This fixes the Ubuntu-built CLI failing on
  Arch/Manjaro with `libusbmuxd-2.0.so.6: cannot open shared object file`.
- Resolve direct and indirect device-library dependencies from the adjacent `lib/`
  directory. Keep the complete extracted release together when moving or installing it.
- Include library version/checksum metadata, licenses and exact distribution sources.
- Check relocated bundle startup on Ubuntu and Arch before uploading release artifacts.
- Show the missing library name or glibc incompatibility in GUI startup errors.

The host still supplies usbmuxd, glibc, TLS and desktop libraries. Existing downloaded
archives are unchanged; this fix takes effect in releases built from the updated workflow.
