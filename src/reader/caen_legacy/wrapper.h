// Wrapper header for bindgen - includes all CAENDigitizer types and functions.
// Uses the system-installed CAENDigitizer header (CAEN Digitizer Library, normally
// under /usr/local/include) — the headers are NOT vendored in this repo so it can
// stay under a permissive (non-GPL) license. Install the CAEN Digitizer Library to build
// with `--features x743`.
#include <CAENDigitizer.h>
