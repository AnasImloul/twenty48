# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `Board::transpose`, which exchanges rows and columns. An evaluation function
  that scores one line at a time needs the columns as rows, and that was the
  only thing an agent had to reimplement from the engine internals.
- `examples/expectimax.rs`, an agent that plays to 32768 in a few milliseconds
  per move. The position evaluation is a table over the 65536 possible lines,
  branches below a probability floor are evaluated rather than expanded, and
  games are played one per core. Search depth and the floor are tunable with
  `--depth` and `--floor`.

## [0.1.0] - 2026-09-30

Initial release.
