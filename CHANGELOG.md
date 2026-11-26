# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-11-26

### Changed

- **Breaking:** `Headers::new` keeps repeated header names apart. The second and later columns
  with the same name are named `name#2`, `name#3`, … (a number already used by another header is
  skipped). Before, only the first column could be read and `raw_payload` dropped the rest.
  Reading by the original name with `require`, `optional`, `list` or `ensure_columns` still
  returns the first column. What changes: `columns()` returns the repeats as `name#2`, … instead
  of `name` again, and `raw_payload` holds every column under those names.

### Added

- `Row::all(name)` returns the values of every column whose original header is `name`, in order,
  trimmed, with empty values left out. A header that was literally `name#2` in the source is not
  counted as a repeat of `name`.
