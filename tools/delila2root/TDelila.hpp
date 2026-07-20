// TDelila.hpp — self-contained reader for DELILA `.delila` data files.
//
// Zero external dependencies (no msgpack-c, no JSON lib, no ROOT): drop this one
// header next to a ROOT macro and `#include "TDelila.hpp"`, or compile it into a
// tool. It parses the MessagePack event records directly and is driven by the
// self-describing schema embedded in the file header (format v3+), falling back
// to a built-in layout for legacy v2 files.
//
// File layout (see src/recorder/format.rs):
//   ["DELILA02"][u32 LE len][MsgPack FileHeader]
//   repeated: [u32 LE len][MsgPack EventDataBatch]
//   [64-byte footer]
//
// Records are rmp-serde "compact" MessagePack: every struct is a positional
// array in field-declaration order. This reader parses each block into a generic
// value tree and uses the schema only to map array positions to field names, so
// adding a field on the Rust side does not break it.
//
// Usage (analysis macro):
//   tdelila::TDelila d("run0003_0000_X743_ThGEM_Test.delila");
//   tdelila::Event ev;
//   while (d.next(ev)) {
//     double t = ev.timestamp_ns();
//     int    ch = ev.channel();
//     int    e  = ev.energy();
//     if (ev.has_waveform()) {
//       const auto& wf = ev.waveform();
//       const std::vector<short>& a0 = wf.analog_probe(1);
//     }
//   }
//
// License: BSD-3-Clause (same as delila-rs).

#ifndef TDELILA_HPP
#define TDELILA_HPP

#include <cstdint>
#include <cstring>
#include <fstream>
#include <map>
#include <stdexcept>
#include <string>
#include <vector>

namespace tdelila {

// ---------------------------------------------------------------------------
// Minimal MessagePack value + reader
// ---------------------------------------------------------------------------
namespace mp {

enum class Kind { Nil, Bool, Int, UInt, F64, Str, Array };

// A decoded MessagePack value. Integers keep their signedness so callers can ask
// for as_i64()/as_u64()/as_f64() without surprises.
struct Value {
  Kind kind = Kind::Nil;
  bool b = false;
  int64_t i = 0;      // valid when kind == Int
  uint64_t u = 0;     // valid when kind == UInt
  double d = 0.0;     // valid when kind == F64
  std::string s;      // valid when kind == Str
  std::vector<Value> arr;  // valid when kind == Array

  bool is_nil() const { return kind == Kind::Nil; }

  int64_t as_i64() const {
    switch (kind) {
      case Kind::Int: return i;
      case Kind::UInt: return static_cast<int64_t>(u);
      case Kind::Bool: return b ? 1 : 0;
      case Kind::F64: return static_cast<int64_t>(d);
      default: return 0;
    }
  }
  uint64_t as_u64() const {
    switch (kind) {
      case Kind::UInt: return u;
      case Kind::Int: return static_cast<uint64_t>(i);
      case Kind::Bool: return b ? 1u : 0u;
      case Kind::F64: return static_cast<uint64_t>(d);
      default: return 0;
    }
  }
  double as_f64() const {
    switch (kind) {
      case Kind::F64: return d;
      case Kind::Int: return static_cast<double>(i);
      case Kind::UInt: return static_cast<double>(u);
      default: return 0.0;
    }
  }
  bool as_bool() const {
    return kind == Kind::Bool ? b : as_u64() != 0;
  }
  const std::string& as_str() const { return s; }
  const std::vector<Value>& as_arr() const { return arr; }
};

// Cursor over an in-memory byte buffer. Throws std::runtime_error on malformed
// or truncated input.
class Reader {
 public:
  Reader(const uint8_t* data, size_t size) : p_(data), end_(data + size) {}

  size_t offset(const uint8_t* base) const { return static_cast<size_t>(p_ - base); }
  const uint8_t* ptr() const { return p_; }
  void seek(const uint8_t* p) { p_ = p; }
  bool at_end() const { return p_ >= end_; }

  // Read one array header, returning element count. Throws if next value is not
  // an array.
  uint32_t read_array_len() {
    uint8_t b = u8();
    if ((b & 0xF0) == 0x90) return b & 0x0F;    // fixarray
    if (b == 0xDC) return be16();               // array16
    if (b == 0xDD) return be32();               // array32
    throw std::runtime_error("expected MessagePack array");
  }

  uint32_t read_map_len() {
    uint8_t b = u8();
    if ((b & 0xF0) == 0x80) return b & 0x0F;    // fixmap
    if (b == 0xDE) return be16();               // map16
    if (b == 0xDF) return be32();               // map32
    throw std::runtime_error("expected MessagePack map");
  }

  // Parse the next value fully (recursively).
  Value read_value() {
    uint8_t b = peek();
    Value v;
    if (b <= 0x7F || b >= 0xE0) {                // fix/neg fixint
      p_++;
      if (b <= 0x7F) { v.kind = Kind::UInt; v.u = b; }
      else { v.kind = Kind::Int; v.i = static_cast<int8_t>(b); }
      return v;
    }
    if ((b & 0xF0) == 0x90 || b == 0xDC || b == 0xDD) {  // array
      uint32_t n = read_array_len();
      v.kind = Kind::Array;
      v.arr.reserve(n);
      for (uint32_t k = 0; k < n; ++k) v.arr.push_back(read_value());
      return v;
    }
    if ((b & 0xE0) == 0xA0 || b == 0xD9 || b == 0xDA || b == 0xDB) {  // str
      v.kind = Kind::Str;
      v.s = read_str();
      return v;
    }
    if ((b & 0xF0) == 0x80 || b == 0xDE || b == 0xDF) {  // map -> parse & discard keys, keep as array of values? keep generic: store as Array of [k,v,...]
      uint32_t n = read_map_len();
      v.kind = Kind::Array;  // maps only appear in the header metadata; caller uses read_str_map instead
      v.arr.reserve(n * 2);
      for (uint32_t k = 0; k < n; ++k) { v.arr.push_back(read_value()); v.arr.push_back(read_value()); }
      return v;
    }
    p_++;
    switch (b) {
      case 0xC0: v.kind = Kind::Nil; return v;
      case 0xC2: v.kind = Kind::Bool; v.b = false; return v;
      case 0xC3: v.kind = Kind::Bool; v.b = true; return v;
      case 0xCC: v.kind = Kind::UInt; v.u = u8(); return v;
      case 0xCD: v.kind = Kind::UInt; v.u = be16(); return v;
      case 0xCE: v.kind = Kind::UInt; v.u = be32(); return v;
      case 0xCF: v.kind = Kind::UInt; v.u = be64(); return v;
      case 0xD0: v.kind = Kind::Int; v.i = static_cast<int8_t>(u8()); return v;
      case 0xD1: v.kind = Kind::Int; v.i = static_cast<int16_t>(be16()); return v;
      case 0xD2: v.kind = Kind::Int; v.i = static_cast<int32_t>(be32()); return v;
      case 0xD3: v.kind = Kind::Int; v.i = static_cast<int64_t>(be64()); return v;
      case 0xCA: { uint32_t r = be32(); float fv; std::memcpy(&fv, &r, 4); v.kind = Kind::F64; v.d = fv; return v; }
      case 0xCB: { uint64_t r = be64(); double dv; std::memcpy(&dv, &r, 8); v.kind = Kind::F64; v.d = dv; return v; }
      case 0xC4: { uint32_t n = u8();  skip_bytes(n); v.kind = Kind::Str; return v; }  // bin -> not used; skip
      case 0xC5: { uint32_t n = be16(); skip_bytes(n); v.kind = Kind::Str; return v; }
      case 0xC6: { uint32_t n = be32(); skip_bytes(n); v.kind = Kind::Str; return v; }
      default:
        throw std::runtime_error("unsupported MessagePack byte 0x" + hex(b));
    }
  }

  // Advance past the next value without materializing it (for lazy fields).
  void skip_value() {
    uint8_t b = peek();
    if (b <= 0x7F || b >= 0xE0) { p_++; return; }
    if ((b & 0xF0) == 0x90 || b == 0xDC || b == 0xDD) {
      uint32_t n = read_array_len();
      for (uint32_t k = 0; k < n; ++k) skip_value();
      return;
    }
    if ((b & 0xE0) == 0xA0 || b == 0xD9 || b == 0xDA || b == 0xDB) { read_str(); return; }
    if ((b & 0xF0) == 0x80 || b == 0xDE || b == 0xDF) {
      uint32_t n = read_map_len();
      for (uint32_t k = 0; k < n; ++k) { skip_value(); skip_value(); }
      return;
    }
    p_++;
    switch (b) {
      case 0xC0: case 0xC2: case 0xC3: return;
      case 0xCC: case 0xD0: skip_bytes(1); return;
      case 0xCD: case 0xD1: skip_bytes(2); return;
      case 0xCE: case 0xD2: case 0xCA: skip_bytes(4); return;
      case 0xCF: case 0xD3: case 0xCB: skip_bytes(8); return;
      case 0xC4: { uint32_t n = u8();  skip_bytes(n); return; }
      case 0xC5: { uint32_t n = be16(); skip_bytes(n); return; }
      case 0xC6: { uint32_t n = be32(); skip_bytes(n); return; }
      default: throw std::runtime_error("skip: unsupported byte 0x" + hex(b));
    }
  }

  std::string read_str() {
    uint8_t b = u8();
    uint32_t n;
    if ((b & 0xE0) == 0xA0) n = b & 0x1F;       // fixstr
    else if (b == 0xD9) n = u8();
    else if (b == 0xDA) n = be16();
    else if (b == 0xDB) n = be32();
    else throw std::runtime_error("expected MessagePack string");
    if (p_ + n > end_) throw std::runtime_error("string overruns buffer");
    std::string s(reinterpret_cast<const char*>(p_), n);
    p_ += n;
    return s;
  }

  // --- Typed decode fast path -------------------------------------------
  // These never build a Value DOM node: read_value() would allocate a Value
  // (std::string + std::vector members, ~96 B) per scalar and per array
  // element. The waveform decode path (512 samples × 19 probe arrays/event)
  // is where that churn dominates, so it decodes straight into caller-owned
  // typed vectors instead. Semantics mirror read_value()/Value::as_* exactly.

  // Decode one integer of any MessagePack int encoding. Throws on a non-int.
  int64_t read_int() {
    uint8_t b = u8();
    if (b <= 0x7F) return b;                       // positive fixint
    if (b >= 0xE0) return static_cast<int8_t>(b);  // negative fixint
    switch (b) {
      case 0xCC: return u8();
      case 0xCD: return be16();
      case 0xCE: return be32();
      case 0xCF: return static_cast<int64_t>(be64());
      case 0xD0: return static_cast<int8_t>(u8());
      case 0xD1: return static_cast<int16_t>(be16());
      case 0xD2: return static_cast<int32_t>(be32());
      case 0xD3: return static_cast<int64_t>(be64());
      default: throw std::runtime_error("expected MessagePack int, got 0x" + hex(b));
    }
  }

  // Decode a float/double, accepting int encodings too (like Value::as_f64).
  double read_f64() {
    uint8_t b = peek();
    if (b == 0xCA) { p_++; uint32_t r = be32(); float fv; std::memcpy(&fv, &r, 4); return fv; }
    if (b == 0xCB) { p_++; uint64_t r = be64(); double dv; std::memcpy(&dv, &r, 8); return dv; }
    return static_cast<double>(read_int());        // int fallthrough
  }

  // Decode a bool, accepting nil (false) and ints (nonzero=true) like as_bool.
  bool read_bool() {
    uint8_t b = peek();
    if (b == 0xC2) { p_++; return false; }
    if (b == 0xC3) { p_++; return true; }
    if (b == 0xC0) { p_++; return false; }         // nil -> false
    return read_int() != 0;
  }

  // Read an array of ints straight into a vector<short>. clear()+reserve keeps
  // the caller's capacity across events (DecodedWaveform reuses its vectors),
  // so the hot path allocates only when a waveform grows past its high-water
  // mark — no per-sample Value construction/destruction.
  void read_i16_array(std::vector<short>& out) {
    uint32_t n = read_array_len();
    out.clear();
    out.reserve(n);
    for (uint32_t k = 0; k < n; ++k) out.push_back(static_cast<short>(read_int()));
  }

  // Digital probes are [u8] on the wire but the ROOT tree stores vector<short>;
  // decode straight into short so we skip the intermediate uint8 vector and its
  // widening copy (×16 branches/event in delila2root). Same loop as the i16
  // reader — the wire values fit in a short either way.
  void read_short_array_from_u8(std::vector<short>& out) { read_i16_array(out); }

  // Read an array of ints into a vector<uint8_t> (the [u8;N] type arrays).
  void read_u8_array(std::vector<uint8_t>& out) {
    uint32_t n = read_array_len();
    out.clear();
    out.reserve(n);
    for (uint32_t k = 0; k < n; ++k) out.push_back(static_cast<uint8_t>(read_int()));
  }

 private:
  uint8_t peek() const { if (p_ >= end_) throw std::runtime_error("truncated MessagePack"); return *p_; }
  uint8_t u8() { if (p_ >= end_) throw std::runtime_error("truncated MessagePack"); return *p_++; }
  uint16_t be16() { uint16_t v = (uint16_t(u8()) << 8); v |= u8(); return v; }
  uint32_t be32() { uint32_t v = 0; for (int k = 0; k < 4; ++k) v = (v << 8) | u8(); return v; }
  uint64_t be64() { uint64_t v = 0; for (int k = 0; k < 8; ++k) v = (v << 8) | u8(); return v; }
  void skip_bytes(uint32_t n) { if (p_ + n > end_) throw std::runtime_error("skip overruns buffer"); p_ += n; }
  static std::string hex(uint8_t b) { char buf[3]; std::snprintf(buf, sizeof(buf), "%02x", b); return buf; }

  const uint8_t* p_;
  const uint8_t* end_;
};

}  // namespace mp

// ---------------------------------------------------------------------------
// Minimal JSON parser (only what the embedded schema needs)
// ---------------------------------------------------------------------------
namespace json {

struct Value {
  enum class T { Null, Bool, Num, Str, Arr, Obj } t = T::Null;
  bool b = false;
  double num = 0.0;
  std::string str;
  std::vector<Value> arr;
  std::map<std::string, Value> obj;
  const Value* find(const std::string& k) const {
    if (t != T::Obj) return nullptr;
    auto it = obj.find(k);
    return it == obj.end() ? nullptr : &it->second;
  }
};

class Parser {
 public:
  explicit Parser(const std::string& s) : s_(s) {}
  Value parse() { skip_ws(); Value v = parse_value(); return v; }

 private:
  Value parse_value() {
    skip_ws();
    char c = peek();
    if (c == '{') return parse_obj();
    if (c == '[') return parse_arr();
    if (c == '"') { Value v; v.t = Value::T::Str; v.str = parse_str(); return v; }
    if (c == 't' || c == 'f') return parse_bool();
    if (c == 'n') { expect("null"); return Value{}; }
    return parse_num();
  }
  Value parse_obj() {
    Value v; v.t = Value::T::Obj; next();  // '{'
    skip_ws();
    if (peek() == '}') { next(); return v; }
    while (true) {
      skip_ws();
      std::string key = parse_str();
      skip_ws(); if (next() != ':') throw std::runtime_error("json: expected ':'");
      v.obj[key] = parse_value();
      skip_ws();
      char c = next();
      if (c == ',') continue;
      if (c == '}') break;
      throw std::runtime_error("json: expected ',' or '}'");
    }
    return v;
  }
  Value parse_arr() {
    Value v; v.t = Value::T::Arr; next();  // '['
    skip_ws();
    if (peek() == ']') { next(); return v; }
    while (true) {
      v.arr.push_back(parse_value());
      skip_ws();
      char c = next();
      if (c == ',') continue;
      if (c == ']') break;
      throw std::runtime_error("json: expected ',' or ']'");
    }
    return v;
  }
  std::string parse_str() {
    if (next() != '"') throw std::runtime_error("json: expected string");
    std::string out;
    while (true) {
      char c = next();
      if (c == '"') break;
      if (c == '\\') {
        char e = next();
        switch (e) {
          case '"': out += '"'; break; case '\\': out += '\\'; break;
          case '/': out += '/'; break; case 'n': out += '\n'; break;
          case 't': out += '\t'; break; case 'r': out += '\r'; break;
          case 'b': out += '\b'; break; case 'f': out += '\f'; break;
          case 'u': { for (int k = 0; k < 4; ++k) next(); out += '?'; break; }
          default: out += e; break;
        }
      } else {
        out += c;
      }
    }
    return out;
  }
  Value parse_bool() {
    Value v; v.t = Value::T::Bool;
    if (peek() == 't') { expect("true"); v.b = true; } else { expect("false"); v.b = false; }
    return v;
  }
  Value parse_num() {
    size_t start = i_;
    while (i_ < s_.size() && (std::strchr("+-0123456789.eE", s_[i_]) != nullptr)) i_++;
    Value v; v.t = Value::T::Num; v.num = std::stod(s_.substr(start, i_ - start));
    return v;
  }
  void expect(const char* lit) { for (const char* q = lit; *q; ++q) if (next() != *q) throw std::runtime_error("json: bad literal"); }
  void skip_ws() { while (i_ < s_.size() && std::strchr(" \t\r\n", s_[i_]) != nullptr) i_++; }
  char peek() { if (i_ >= s_.size()) throw std::runtime_error("json: eof"); return s_[i_]; }
  char next() { if (i_ >= s_.size()) throw std::runtime_error("json: eof"); return s_[i_++]; }

  const std::string& s_;
  size_t i_ = 0;
};

inline Value parse(const std::string& s) { return Parser(s).parse(); }

}  // namespace json

// ---------------------------------------------------------------------------
// Schema (ordered field name + type-tag per record type)
// ---------------------------------------------------------------------------
struct Field {
  std::string name;
  std::string tag;  // u8|u16|u32|u64|i16|f64|bool | [T] | [T;N] | ?T | Name
};

class Schema {
 public:
  // Build the schema from the header's embedded JSON, or fall back to the
  // built-in v2/v3 layout when `event_schema` is absent (legacy files).
  static Schema from_header_json(const std::string& schema_json) {
    Schema sc;
    if (schema_json.empty()) { sc.build_default(); return sc; }
    try {
      json::Value root = json::parse(schema_json);
      const json::Value* rec = root.find("record");
      sc.record_ = (rec && rec->t == json::Value::T::Str) ? rec->str : "EventDataBatch";
      const json::Value* types = root.find("types");
      if (!types || types->t != json::Value::T::Obj) { sc.build_default(); return sc; }
      for (const auto& kv : types->obj) {
        std::vector<Field> fields;
        if (kv.second.t == json::Value::T::Arr) {
          for (const auto& fv : kv.second.arr) {
            const json::Value* n = fv.find("name");
            const json::Value* t = fv.find("type");
            if (n && t) fields.push_back({n->str, t->str});
          }
        }
        sc.types_[kv.first] = std::move(fields);
      }
    } catch (const std::exception&) {
      sc.build_default();
    }
    return sc;
  }

  const std::string& record() const { return record_; }
  bool has_type(const std::string& name) const { return types_.count(name) != 0; }
  const std::vector<Field>& fields(const std::string& name) const {
    static const std::vector<Field> empty;
    auto it = types_.find(name);
    return it == types_.end() ? empty : it->second;
  }
  // Index of a named field in a type, or -1.
  int index_of(const std::string& type, const std::string& field) const {
    const auto& fs = fields(type);
    for (size_t k = 0; k < fs.size(); ++k) if (fs[k].name == field) return static_cast<int>(k);
    return -1;
  }

 private:
  void build_default() {
    record_ = "EventDataBatch";
    types_["EventDataBatch"] = {
        {"source_id", "u32"}, {"sequence_number", "u64"}, {"timestamp", "u64"}, {"events", "[EventData]"}};
    types_["EventData"] = {
        {"module", "u8"}, {"channel", "u8"}, {"energy", "u16"}, {"energy_short", "u16"},
        {"timestamp_ns", "f64"}, {"flags", "u64"}, {"user_info", "[u64;4]"}, {"waveform", "?Waveform"}};
    std::vector<Field> wf = {{"analog_probe1", "[i16]"}, {"analog_probe2", "[i16]"}, {"analog_probe3", "[i16]"}};
    for (int k = 1; k <= 16; ++k) wf.push_back({"digital_probe" + std::to_string(k), "[u8]"});
    wf.push_back({"time_resolution", "u8"});
    wf.push_back({"trigger_threshold", "u16"});
    wf.push_back({"ns_per_sample", "f64"});
    wf.push_back({"analog_probe1_is_signed", "bool"});
    wf.push_back({"analog_probe2_is_signed", "bool"});
    wf.push_back({"analog_probe3_is_signed", "bool"});
    wf.push_back({"analog_probe_type", "[u8;3]"});
    wf.push_back({"digital_probe_type", "[u8;16]"});
    types_["Waveform"] = std::move(wf);
  }

  std::string record_ = "EventDataBatch";
  std::map<std::string, std::vector<Field>> types_;
};

// ---------------------------------------------------------------------------
// Header / footer
// ---------------------------------------------------------------------------
struct FileHeader {
  uint32_t version = 0;
  uint32_t run_number = 0;
  std::string exp_name;
  uint32_t file_sequence = 0;
  uint64_t file_start_time_ns = 0;
  std::string comment;
  std::string event_schema;  // metadata["event_schema"], empty for v2
};

struct FileFooter {
  bool present = false;
  uint64_t total_events = 0;
  uint64_t data_bytes = 0;
  double first_event_time_ns = 0;
  double last_event_time_ns = 0;
  bool write_complete = false;
};

// ---------------------------------------------------------------------------
// Waveform (materialized lazily from an EventData record)
// ---------------------------------------------------------------------------
class Waveform {
 public:
  Waveform() = default;
  explicit Waveform(mp::Value v, const Schema* sc) : v_(std::move(v)), sc_(sc) {}

  bool valid() const { return v_.kind == mp::Kind::Array; }

  // Generic field access by schema name (returns the raw MessagePack value).
  const mp::Value* field(const std::string& name) const {
    if (!sc_) return nullptr;
    int idx = sc_->index_of("Waveform", name);
    if (idx < 0 || static_cast<size_t>(idx) >= v_.arr.size()) return nullptr;
    return &v_.arr[static_cast<size_t>(idx)];
  }

  // analog_probe(1..3) as int16 samples.
  std::vector<short> analog_probe(int n) const {
    return int16_array("analog_probe" + std::to_string(n));
  }
  // digital_probe(1..16) as 0/1 bytes.
  std::vector<uint8_t> digital_probe(int n) const {
    return uint8_array("digital_probe" + std::to_string(n));
  }
  double ns_per_sample() const { const mp::Value* f = field("ns_per_sample"); return f ? f->as_f64() : 0.0; }
  int trigger_threshold() const { const mp::Value* f = field("trigger_threshold"); return f ? int(f->as_u64()) : 0; }
  bool analog_probe_is_signed(int n) const {
    const mp::Value* f = field("analog_probe" + std::to_string(n) + "_is_signed");
    return f ? f->as_bool() : false;
  }

  std::vector<short> int16_array(const std::string& name) const {
    std::vector<short> out; const mp::Value* f = field(name);
    if (f && f->kind == mp::Kind::Array) { out.reserve(f->arr.size()); for (const auto& e : f->arr) out.push_back(static_cast<short>(e.as_i64())); }
    return out;
  }
  std::vector<uint8_t> uint8_array(const std::string& name) const {
    std::vector<uint8_t> out; const mp::Value* f = field(name);
    if (f && f->kind == mp::Kind::Array) { out.reserve(f->arr.size()); for (const auto& e : f->arr) out.push_back(static_cast<uint8_t>(e.as_u64())); }
    return out;
  }

 private:
  mp::Value v_;
  const Schema* sc_ = nullptr;
};

// ---------------------------------------------------------------------------
// Typed waveform decode (no DOM)
// ---------------------------------------------------------------------------
// One entry of the per-file decode plan: it maps a Waveform field *position*
// to an action, so decode_waveform() never does a schema name lookup per event
// (unlike the generic Waveform::field() DOM path). The plan is built once from
// schema_.fields("Waveform"); unknown/future fields become Skip, preserving v3
// forward compatibility.
struct WfTarget {
  enum class Kind {
    Skip,              // unknown/future field -> skip_value()
    AnalogK,           // analog_probe(k+1)          -> analog[k]   ([i16])
    DigitalK,          // digital_probe(k+1)         -> digital[k]  ([u8]->short)
    NsPerSample,       // ns_per_sample              -> f64
    TriggerThreshold,  // trigger_threshold          -> u16
    TimeResolution,    // time_resolution            -> u8
    AnalogSignedK,     // analog_probe(k+1)_is_signed-> bool
    AnalogTypeArr,     // analog_probe_type          -> [u8;3]
    DigitalTypeArr,    // digital_probe_type         -> [u8;16]
  };
  Kind kind = Kind::Skip;
  int k = 0;  // probe index (0-based) for AnalogK/DigitalK/AnalogSignedK
};

// Typed decode target for one waveform: same data the DOM Waveform exposes, but
// laid out as plain vectors/arrays so a converter can point ROOT branches at it
// and reuse it across events (clear() keeps every vector's capacity).
struct DecodedWaveform {
  std::vector<short> analog[3];
  std::vector<short> digital[16];
  double ns_per_sample = 0.0;
  uint16_t trigger_threshold = 0;
  uint8_t time_resolution = 0;
  bool analog_is_signed[3] = {false, false, false};
  uint8_t analog_type[3] = {0, 0, 0};
  uint8_t digital_type[16] = {0};

  // Reset for reuse: empty every vector (retaining capacity) and zero scalars.
  void clear() {
    for (int k = 0; k < 3; ++k) { analog[k].clear(); analog_is_signed[k] = false; analog_type[k] = 0; }
    for (int k = 0; k < 16; ++k) { digital[k].clear(); digital_type[k] = 0; }
    ns_per_sample = 0.0;
    trigger_threshold = 0;
    time_resolution = 0;
  }
};

// ---------------------------------------------------------------------------
// Event
// ---------------------------------------------------------------------------
class TDelila;  // fwd

class Event {
 public:
  int module() const { return get_u("module"); }
  int channel() const { return get_u("channel"); }
  int energy() const { return get_u("energy"); }
  int energy_short() const { return get_u("energy_short"); }
  double timestamp_ns() const { const mp::Value* f = field("timestamp_ns"); return f ? f->as_f64() : 0.0; }
  uint64_t flags() const { const mp::Value* f = field("flags"); return f ? f->as_u64() : 0; }
  uint64_t user_info(int slot) const {
    const mp::Value* f = field("user_info");
    if (f && f->kind == mp::Kind::Array && slot >= 0 && static_cast<size_t>(slot) < f->arr.size())
      return f->arr[static_cast<size_t>(slot)].as_u64();
    return 0;
  }

  bool has_waveform() const { return has_wf_; }

  // Lazily materialized from the copied waveform bytes; cached after first call.
  // The bytes are owned by the Event, so this is safe to call at any time.
  const Waveform& waveform() const {
    if (!wf_built_) {
      if (has_wf_ && !wf_bytes_.empty()) {
        mp::Reader r(wf_bytes_.data(), wf_bytes_.size());
        wf_ = Waveform(r.read_value(), sc_);
      }
      wf_built_ = true;
    }
    return wf_;
  }

  // Typed decode into caller-owned storage — the converter's hot path. Unlike
  // waveform() it builds no DOM: it walks wf_bytes_ once, driven by the file's
  // precomputed plan_ (field position -> action), decoding each array straight
  // into `out`'s reused vectors. Returns false (after out.clear()) when there
  // is no waveform. Semantically identical to reading waveform()'s accessors.
  bool decode_waveform(DecodedWaveform& out) const {
    out.clear();
    if (!has_wf_ || wf_bytes_.empty() || !plan_) return false;
    mp::Reader r(wf_bytes_.data(), wf_bytes_.size());
    // Decode only min(actual array len, plan size) fields: a short/future
    // waveform must not read past its own elements, and any plan entry beyond
    // the wire array stays cleared (nil handling / v3 forward compat).
    uint32_t n = r.read_array_len();
    const std::vector<WfTarget>& plan = *plan_;
    size_t limit = static_cast<size_t>(n) < plan.size() ? static_cast<size_t>(n) : plan.size();
    for (size_t i = 0; i < limit; ++i) {
      const WfTarget& t = plan[i];
      switch (t.kind) {
        case WfTarget::Kind::AnalogK:      r.read_i16_array(out.analog[t.k]); break;
        case WfTarget::Kind::DigitalK:     r.read_short_array_from_u8(out.digital[t.k]); break;
        case WfTarget::Kind::NsPerSample:  out.ns_per_sample = r.read_f64(); break;
        case WfTarget::Kind::TriggerThreshold: out.trigger_threshold = static_cast<uint16_t>(r.read_int()); break;
        case WfTarget::Kind::TimeResolution:   out.time_resolution = static_cast<uint8_t>(r.read_int()); break;
        case WfTarget::Kind::AnalogSignedK:    out.analog_is_signed[t.k] = r.read_bool(); break;
        case WfTarget::Kind::AnalogTypeArr: {  // [u8;3] — decode inline into the fixed array (no per-event heap)
          uint32_t m = r.read_array_len();
          for (uint32_t k = 0; k < m; ++k) { int64_t v = r.read_int(); if (k < 3) out.analog_type[k] = static_cast<uint8_t>(v); }
          break;
        }
        case WfTarget::Kind::DigitalTypeArr: {  // [u8;16]
          uint32_t m = r.read_array_len();
          for (uint32_t k = 0; k < m; ++k) { int64_t v = r.read_int(); if (k < 16) out.digital_type[k] = static_cast<uint8_t>(v); }
          break;
        }
        case WfTarget::Kind::Skip:         r.skip_value(); break;
      }
    }
    return true;
  }

  // Generic access to any scalar EventData field by schema name (the waveform
  // slot is a placeholder here — use waveform()).
  const mp::Value* field(const std::string& name) const {
    if (!sc_) return nullptr;
    int idx = sc_->index_of("EventData", name);
    if (idx < 0 || static_cast<size_t>(idx) >= fields_.arr.size()) return nullptr;
    return &fields_.arr[static_cast<size_t>(idx)];
  }

 private:
  friend class TDelila;
  int get_u(const std::string& name) const { const mp::Value* f = field(name); return f ? static_cast<int>(f->as_u64()) : 0; }

  mp::Value fields_;          // EventData scalar fields (waveform slot = nil placeholder)
  const Schema* sc_ = nullptr;
  const std::vector<WfTarget>* plan_ = nullptr;  // file-owned waveform decode plan
  bool has_wf_ = false;
  std::vector<uint8_t> wf_bytes_;  // raw MessagePack bytes of the waveform (owned, lazy)
  mutable bool wf_built_ = false;
  mutable Waveform wf_;
};

// ---------------------------------------------------------------------------
// TDelila — file reader
// ---------------------------------------------------------------------------
class TDelila {
 public:
  explicit TDelila(const std::string& path) { open(path); }

  bool good() const { return good_; }
  const std::string& error() const { return error_; }
  const FileHeader& header() const { return header_; }
  const FileFooter& footer() const { return footer_; }
  const Schema& schema() const { return schema_; }

  // Advance to the next event. Returns false at end of data / on error.
  bool next(Event& out) {
    while (good_) {
      if (events_remaining_ > 0) {
        parse_one_event(out);
        events_remaining_--;
        events_returned_++;
        return true;
      }
      if (!load_next_block()) return false;  // no more blocks
    }
    return false;
  }

  uint64_t events_returned() const { return events_returned_; }

 private:
  void open(const std::string& path) {
    f_.open(path, std::ios::binary);
    if (!f_) { fail("cannot open file: " + path); return; }
    f_.seekg(0, std::ios::end);
    file_size_ = static_cast<uint64_t>(f_.tellg());
    f_.seekg(0, std::ios::beg);
    if (file_size_ < 12 + 64) { fail("file too small"); return; }

    // Header: magic(8) + u32 len + msgpack.
    uint8_t magic[8];
    f_.read(reinterpret_cast<char*>(magic), 8);
    if (std::memcmp(magic, "DELILA02", 8) != 0) { fail("bad file magic"); return; }
    uint32_t hlen = read_u32le();
    std::vector<uint8_t> hbuf(hlen);
    f_.read(reinterpret_cast<char*>(hbuf.data()), hlen);
    parse_header(hbuf);

    header_size_ = 8u + 4u + hlen;
    data_end_ = file_size_ - 64;  // footer is fixed 64 bytes

    read_footer();

    schema_ = Schema::from_header_json(header_.event_schema);
    wf_index_ = schema_.index_of("EventData", "waveform");
    build_wf_plan();
    block_pos_ = header_size_;
    good_ = true;
  }

  // Build the per-file waveform decode plan from the schema (once, in open()).
  // Field names are matched exactly against the v2/v3 Waveform layout; anything
  // unrecognized (a field the Rust side adds later) maps to Skip so decode
  // stays position-correct without knowing the new field.
  void build_wf_plan() {
    wf_plan_.clear();
    const auto& fs = schema_.fields("Waveform");
    wf_plan_.reserve(fs.size());
    for (const auto& fd : fs) {
      WfTarget t;  // defaults to Skip
      const std::string& nm = fd.name;
      bool matched = false;
      for (int k = 0; k < 3 && !matched; ++k) {
        if (nm == "analog_probe" + std::to_string(k + 1)) { t.kind = WfTarget::Kind::AnalogK; t.k = k; matched = true; }
        else if (nm == "analog_probe" + std::to_string(k + 1) + "_is_signed") { t.kind = WfTarget::Kind::AnalogSignedK; t.k = k; matched = true; }
      }
      for (int k = 0; k < 16 && !matched; ++k)
        if (nm == "digital_probe" + std::to_string(k + 1)) { t.kind = WfTarget::Kind::DigitalK; t.k = k; matched = true; }
      if (!matched) {
        if (nm == "time_resolution") t.kind = WfTarget::Kind::TimeResolution;
        else if (nm == "trigger_threshold") t.kind = WfTarget::Kind::TriggerThreshold;
        else if (nm == "ns_per_sample") t.kind = WfTarget::Kind::NsPerSample;
        else if (nm == "analog_probe_type") t.kind = WfTarget::Kind::AnalogTypeArr;
        else if (nm == "digital_probe_type") t.kind = WfTarget::Kind::DigitalTypeArr;
        // else: unknown field -> Skip (default)
      }
      wf_plan_.push_back(t);
    }
  }

  void parse_header(const std::vector<uint8_t>& buf) {
    mp::Reader r(buf.data(), buf.size());
    // FileHeader positional array: version, run_number, exp_name, file_sequence,
    // file_start_time_ns, comment, sort_margin_ratio, is_sorted, source_ids, metadata
    uint32_t n = r.read_array_len();
    mp::Value version = r.read_value();
    mp::Value run_number = r.read_value();
    mp::Value exp_name = r.read_value();
    mp::Value file_sequence = r.read_value();
    mp::Value start_time = r.read_value();
    mp::Value comment = (n > 5) ? r.read_value() : mp::Value();
    // remaining: sort_margin_ratio(6), is_sorted(7), source_ids(8), metadata(9)
    if (n > 6) r.skip_value();  // sort_margin_ratio
    if (n > 7) r.skip_value();  // is_sorted
    if (n > 8) r.skip_value();  // source_ids
    header_.version = static_cast<uint32_t>(version.as_u64());
    header_.run_number = static_cast<uint32_t>(run_number.as_u64());
    header_.exp_name = exp_name.kind == mp::Kind::Str ? exp_name.s : "";
    header_.file_sequence = static_cast<uint32_t>(file_sequence.as_u64());
    header_.file_start_time_ns = start_time.as_u64();
    header_.comment = comment.kind == mp::Kind::Str ? comment.s : "";
    if (n > 9) {
      uint32_t m = r.read_map_len();
      for (uint32_t k = 0; k < m; ++k) {
        std::string key = r.read_str();
        mp::Value val = r.read_value();
        if (key == "event_schema" && val.kind == mp::Kind::Str) header_.event_schema = val.s;
      }
    }
  }

  void read_footer() {
    std::streampos save = f_.tellg();
    f_.seekg(static_cast<std::streamoff>(file_size_ - 64), std::ios::beg);
    uint8_t fb[64];
    f_.read(reinterpret_cast<char*>(fb), 64);
    if (std::memcmp(fb, "DLEND002", 8) == 0) {
      footer_.present = true;
      footer_.total_events = le64(fb + 16);
      footer_.data_bytes = le64(fb + 24);
      footer_.first_event_time_ns = le_f64(fb + 32);
      footer_.last_event_time_ns = le_f64(fb + 40);
      footer_.write_complete = fb[56] == 1;
    }
    f_.seekg(save);
  }

  // Read the next data block and position the cursor at its first event.
  // Returns false when the data region ends. Waveforms are NOT parsed here.
  bool load_next_block() {
    events_remaining_ = 0;
    while (block_pos_ + 4 <= data_end_) {
      f_.seekg(static_cast<std::streamoff>(block_pos_), std::ios::beg);
      uint32_t blen = read_u32le();
      if (blen == 0 || block_pos_ + 4 + blen > data_end_) return false;
      block_buf_.resize(blen);
      f_.read(reinterpret_cast<char*>(block_buf_.data()), blen);
      block_pos_ += 4 + blen;
      try {
        mp::Reader r(block_buf_.data(), block_buf_.size());
        r.read_array_len();  // EventDataBatch fields (4)
        r.skip_value();      // source_id
        r.skip_value();      // sequence_number
        r.skip_value();      // timestamp
        events_remaining_ = r.read_array_len();  // events array; cursor at first event
        ev_cursor_ = r.offset(block_buf_.data());
        if (events_remaining_ == 0) continue;  // empty batch — try next block
        return true;
      } catch (const std::exception& e) {
        fail(std::string("block parse error: ") + e.what());
        return false;
      }
    }
    return false;
  }

  // Parse one EventData from the current block cursor into `out`. Scalar fields
  // are decoded; the waveform slot's raw bytes are copied for lazy parsing.
  void parse_one_event(Event& out) {
    mp::Reader r(block_buf_.data(), block_buf_.size());
    r.seek(block_buf_.data() + ev_cursor_);
    uint32_t len = r.read_array_len();
    out.fields_.kind = mp::Kind::Array;
    out.fields_.arr.clear();
    out.fields_.arr.reserve(len);
    out.sc_ = &schema_;
    out.plan_ = &wf_plan_;
    out.has_wf_ = false;
    out.wf_bytes_.clear();
    out.wf_built_ = false;
    out.wf_ = Waveform();
    for (uint32_t i = 0; i < len; ++i) {
      if (static_cast<int>(i) == wf_index_) {
        const uint8_t* start = r.ptr();
        r.skip_value();
        const uint8_t* end = r.ptr();
        out.fields_.arr.push_back(mp::Value());  // nil placeholder keeps indices aligned
        if (!(end - start == 1 && *start == 0xC0)) {
          out.has_wf_ = true;
          out.wf_bytes_.assign(start, end);
        }
      } else {
        out.fields_.arr.push_back(r.read_value());
      }
    }
    ev_cursor_ = r.offset(block_buf_.data());
  }

  uint32_t read_u32le() {
    uint8_t b[4];
    f_.read(reinterpret_cast<char*>(b), 4);
    return uint32_t(b[0]) | (uint32_t(b[1]) << 8) | (uint32_t(b[2]) << 16) | (uint32_t(b[3]) << 24);
  }
  static uint64_t le64(const uint8_t* p) {
    uint64_t v = 0; for (int k = 7; k >= 0; --k) v = (v << 8) | p[k]; return v;
  }
  static double le_f64(const uint8_t* p) { uint64_t r = le64(p); double d; std::memcpy(&d, &r, 8); return d; }

  void fail(const std::string& msg) { good_ = false; error_ = msg; }

  std::ifstream f_;
  bool good_ = false;
  std::string error_;
  uint64_t file_size_ = 0;
  uint64_t header_size_ = 0;
  uint64_t data_end_ = 0;
  uint64_t block_pos_ = 0;
  FileHeader header_;
  FileFooter footer_;
  Schema schema_;
  std::vector<WfTarget> wf_plan_;  // waveform decode plan (built once in open())
  std::vector<uint8_t> block_buf_;
  size_t ev_cursor_ = 0;       // byte offset of next event within block_buf_
  size_t events_remaining_ = 0;
  int wf_index_ = -1;          // EventData index of the waveform field (or -1)
  uint64_t events_returned_ = 0;
};

}  // namespace tdelila

#endif  // TDELILA_HPP
