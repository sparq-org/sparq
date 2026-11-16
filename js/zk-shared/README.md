# Shared proof package support

Research grade and not externally audited. This directory owns the proof backend
settings, pinned dependency checks, CRS handling and offline test wrapper used by
the Node proof packages. Their maintainer builds copy these sources into each
package's ignored `internal` directory. The package tests check those copies for
drift; tarballs contain the copies so installations are self-contained.
