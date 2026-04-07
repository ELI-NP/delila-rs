// delila2root — Convert .delila files to time-sorted ROOT TTree
//
// One event per TTree entry (scalar branches) for simple single-loop reading.
//
// Usage:
//   ./delila2root -o output.root data/run0018_*.delila
//
// Algorithm: Per-file sort + two-pointer merge (sliding window)
//   1. Read footers → sort files by first_event_time
//   2. For each file: read all events → sort by timestamp
//   3. Two-pointer merge with carry_over → TTree::Fill for safe events
//   4. Carry unsafe tail to next iteration
//
// MsgPack parser adapted from macros/read_delila.C

#include <TFile.h>
#include <TTree.h>
#include <Compression.h>
#include <TROOT.h>

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <iostream>
#include <limits>
#include <memory>
#include <string>
#include <vector>

// ============================================================================
// File format constants
// ============================================================================
static const char* FILE_MAGIC = "DELILA02";
static const char* FOOTER_MAGIC = "DLEND002";
static constexpr size_t FOOTER_SIZE = 64;

// ============================================================================
// Data structures
// ============================================================================
struct WaveformData {
    std::vector<Short_t> analog_probe1;
    std::vector<Short_t> analog_probe2;
    std::vector<UChar_t> digital_probe1;
    std::vector<UChar_t> digital_probe2;
    std::vector<UChar_t> digital_probe3;
    std::vector<UChar_t> digital_probe4;
    UChar_t time_resolution = 0;
    UShort_t trigger_threshold = 0;
    Double_t ns_per_sample = 0.0;
};

struct Event {
    double timestamp_ns;
    uint64_t flags;
    uint16_t energy;
    uint16_t energy_short;
    uint8_t module;
    uint8_t channel;
    std::unique_ptr<WaveformData> waveform;  // nullptr if no waveform
};

struct FileInfo {
    std::string path;
    double first_event_time_ns;
    double last_event_time_ns;
    uint64_t total_events;
};

// ============================================================================
// Binary readers
// ============================================================================
static uint32_t read_u32_le(std::ifstream& f) {
    uint8_t buf[4];
    f.read(reinterpret_cast<char*>(buf), 4);
    return buf[0] | (buf[1] << 8) | (buf[2] << 16) |
           (static_cast<uint32_t>(buf[3]) << 24);
}

static uint64_t read_u64_le(std::ifstream& f) {
    uint8_t buf[8];
    f.read(reinterpret_cast<char*>(buf), 8);
    return static_cast<uint64_t>(buf[0]) |
           (static_cast<uint64_t>(buf[1]) << 8) |
           (static_cast<uint64_t>(buf[2]) << 16) |
           (static_cast<uint64_t>(buf[3]) << 24) |
           (static_cast<uint64_t>(buf[4]) << 32) |
           (static_cast<uint64_t>(buf[5]) << 40) |
           (static_cast<uint64_t>(buf[6]) << 48) |
           (static_cast<uint64_t>(buf[7]) << 56);
}

static double read_f64_le(std::ifstream& f) {
    uint64_t bits = read_u64_le(f);
    double result;
    std::memcpy(&result, &bits, sizeof(double));
    return result;
}

// ============================================================================
// MsgPack parser (from macros/read_delila.C)
// ============================================================================
class MsgPackParser {
   public:
    MsgPackParser(const std::vector<uint8_t>& data,
                  uint16_t energy_min = 0)
        : data_(data), pos_(0), energy_min_(energy_min) {}

    bool parse_batch(std::vector<Event>& events) {
        size_t batch_size;
        if (!read_array_header(batch_size) || batch_size != 4) return false;

        uint64_t tmp;
        if (!read_uint(tmp)) return false;  // source_id
        if (!read_uint(tmp)) return false;  // sequence_number
        if (!read_uint(tmp)) return false;  // timestamp

        size_t num_events;
        if (!read_array_header(num_events)) return false;

        events.reserve(events.size() + num_events);
        for (size_t i = 0; i < num_events; i++) {
            if (!parse_event(events)) return false;
        }
        return true;
    }

   private:
    bool parse_event(std::vector<Event>& events) {
        size_t ev_size;
        if (!read_array_header(ev_size)) return false;
        if (ev_size != 6 && ev_size != 7) return false;

        uint64_t mod, ch, e, es, fl;
        double ts;
        if (!read_uint(mod)) return false;
        if (!read_uint(ch)) return false;
        if (!read_uint(e)) return false;
        if (!read_uint(es)) return false;
        if (!read_float64(ts)) return false;
        if (!read_uint(fl)) return false;

        // Parse waveform data if present
        std::unique_ptr<WaveformData> wf;
        if (ev_size == 7) {
            wf = std::make_unique<WaveformData>();
            if (!parse_waveform(*wf)) return false;
        }

        // Energy threshold filter (waveform auto-freed if filtered)
        if (static_cast<uint16_t>(e) < energy_min_) {
            return true;
        }

        Event ev;
        ev.timestamp_ns = ts;
        ev.flags = fl;
        ev.energy = static_cast<uint16_t>(e);
        ev.energy_short = static_cast<uint16_t>(es);
        ev.module = static_cast<uint8_t>(mod);
        ev.channel = static_cast<uint8_t>(ch);
        ev.waveform = std::move(wf);
        events.push_back(std::move(ev));

        return true;
    }

    bool read_array_header(size_t& size) {
        if (pos_ >= data_.size()) return false;
        uint8_t b = data_[pos_++];
        if ((b & 0xf0) == 0x90) { size = b & 0x0f; return true; }
        if (b == 0xdc && pos_ + 2 <= data_.size()) {
            size = (data_[pos_] << 8) | data_[pos_ + 1];
            pos_ += 2; return true;
        }
        if (b == 0xdd && pos_ + 4 <= data_.size()) {
            size = (static_cast<uint32_t>(data_[pos_]) << 24) |
                   (static_cast<uint32_t>(data_[pos_ + 1]) << 16) |
                   (static_cast<uint32_t>(data_[pos_ + 2]) << 8) |
                   static_cast<uint32_t>(data_[pos_ + 3]);
            pos_ += 4; return true;
        }
        return false;
    }

    bool read_uint(uint64_t& val) {
        if (pos_ >= data_.size()) return false;
        uint8_t b = data_[pos_++];
        if (b <= 0x7f) { val = b; return true; }
        if (b == 0xcc && pos_ + 1 <= data_.size()) {
            val = data_[pos_++]; return true;
        }
        if (b == 0xcd && pos_ + 2 <= data_.size()) {
            val = (data_[pos_] << 8) | data_[pos_ + 1];
            pos_ += 2; return true;
        }
        if (b == 0xce && pos_ + 4 <= data_.size()) {
            val = (static_cast<uint32_t>(data_[pos_]) << 24) |
                  (static_cast<uint32_t>(data_[pos_ + 1]) << 16) |
                  (static_cast<uint32_t>(data_[pos_ + 2]) << 8) |
                  static_cast<uint32_t>(data_[pos_ + 3]);
            pos_ += 4; return true;
        }
        if (b == 0xcf && pos_ + 8 <= data_.size()) {
            val = (static_cast<uint64_t>(data_[pos_]) << 56) |
                  (static_cast<uint64_t>(data_[pos_ + 1]) << 48) |
                  (static_cast<uint64_t>(data_[pos_ + 2]) << 40) |
                  (static_cast<uint64_t>(data_[pos_ + 3]) << 32) |
                  (static_cast<uint64_t>(data_[pos_ + 4]) << 24) |
                  (static_cast<uint64_t>(data_[pos_ + 5]) << 16) |
                  (static_cast<uint64_t>(data_[pos_ + 6]) << 8) |
                  static_cast<uint64_t>(data_[pos_ + 7]);
            pos_ += 8; return true;
        }
        return false;
    }

    bool read_float64(double& val) {
        if (pos_ >= data_.size()) return false;
        uint8_t b = data_[pos_++];
        if (b == 0xcb && pos_ + 8 <= data_.size()) {
            uint64_t bits =
                (static_cast<uint64_t>(data_[pos_]) << 56) |
                (static_cast<uint64_t>(data_[pos_ + 1]) << 48) |
                (static_cast<uint64_t>(data_[pos_ + 2]) << 40) |
                (static_cast<uint64_t>(data_[pos_ + 3]) << 32) |
                (static_cast<uint64_t>(data_[pos_ + 4]) << 24) |
                (static_cast<uint64_t>(data_[pos_ + 5]) << 16) |
                (static_cast<uint64_t>(data_[pos_ + 6]) << 8) |
                static_cast<uint64_t>(data_[pos_ + 7]);
            pos_ += 8;
            std::memcpy(&val, &bits, sizeof(double));
            return true;
        }
        return false;
    }

    bool read_int(int64_t& val) {
        if (pos_ >= data_.size()) return false;
        uint8_t b = data_[pos_];

        // positive fixint (0x00 - 0x7f)
        if (b <= 0x7f) {
            pos_++;
            val = b;
            return true;
        }
        // negative fixint (0xe0 - 0xff)
        if (b >= 0xe0) {
            pos_++;
            val = static_cast<int8_t>(b);
            return true;
        }
        // int8 (0xd0)
        if (b == 0xd0 && pos_ + 2 <= data_.size()) {
            pos_++;
            val = static_cast<int8_t>(data_[pos_++]);
            return true;
        }
        // int16 (0xd1)
        if (b == 0xd1 && pos_ + 3 <= data_.size()) {
            pos_++;
            val = static_cast<int16_t>((data_[pos_] << 8) | data_[pos_ + 1]);
            pos_ += 2;
            return true;
        }
        // int32 (0xd2)
        if (b == 0xd2 && pos_ + 5 <= data_.size()) {
            pos_++;
            val = static_cast<int32_t>(
                (static_cast<uint32_t>(data_[pos_]) << 24) |
                (static_cast<uint32_t>(data_[pos_ + 1]) << 16) |
                (static_cast<uint32_t>(data_[pos_ + 2]) << 8) |
                static_cast<uint32_t>(data_[pos_ + 3]));
            pos_ += 4;
            return true;
        }
        // uint types (for positive values stored as unsigned)
        uint64_t uval;
        if (read_uint(uval)) {
            val = static_cast<int64_t>(uval);
            return true;
        }
        return false;
    }

    bool read_i16_array(std::vector<Short_t>& arr) {
        size_t size;
        if (!read_array_header(size)) return false;

        arr.resize(size);
        for (size_t i = 0; i < size; i++) {
            int64_t val;
            if (!read_int(val)) return false;
            arr[i] = static_cast<Short_t>(val);
        }
        return true;
    }

    bool read_bin(std::vector<UChar_t>& arr) {
        if (pos_ >= data_.size()) return false;
        uint8_t b = data_[pos_++];
        size_t size = 0;

        if (b == 0xc4 && pos_ + 1 <= data_.size()) {
            // bin8
            size = data_[pos_++];
        } else if (b == 0xc5 && pos_ + 2 <= data_.size()) {
            // bin16
            size = (data_[pos_] << 8) | data_[pos_ + 1];
            pos_ += 2;
        } else if (b == 0xc6 && pos_ + 4 <= data_.size()) {
            // bin32
            size = (static_cast<uint32_t>(data_[pos_]) << 24) |
                   (static_cast<uint32_t>(data_[pos_ + 1]) << 16) |
                   (static_cast<uint32_t>(data_[pos_ + 2]) << 8) |
                   static_cast<uint32_t>(data_[pos_ + 3]);
            pos_ += 4;
        } else {
            return false;
        }

        if (pos_ + size > data_.size()) return false;
        arr.assign(data_.begin() + pos_, data_.begin() + pos_ + size);
        pos_ += size;
        return true;
    }

    bool read_u8_array(std::vector<UChar_t>& arr) {
        if (pos_ >= data_.size()) return false;
        uint8_t b = data_[pos_];

        // Check for binary format first
        if (b == 0xc4 || b == 0xc5 || b == 0xc6) {
            return read_bin(arr);
        }

        // Otherwise, it's an array
        size_t size;
        if (!read_array_header(size)) return false;

        arr.resize(size);
        for (size_t i = 0; i < size; i++) {
            uint64_t val;
            if (!read_uint(val)) return false;
            arr[i] = static_cast<UChar_t>(val);
        }
        return true;
    }

    bool parse_waveform(WaveformData& wf) {
        size_t wf_size;
        if (!read_array_header(wf_size) || (wf_size != 8 && wf_size != 9)) {
            return false;
        }

        if (!read_i16_array(wf.analog_probe1)) return false;
        if (!read_i16_array(wf.analog_probe2)) return false;
        if (!read_u8_array(wf.digital_probe1)) return false;
        if (!read_u8_array(wf.digital_probe2)) return false;
        if (!read_u8_array(wf.digital_probe3)) return false;
        if (!read_u8_array(wf.digital_probe4)) return false;

        uint64_t tmp;
        if (!read_uint(tmp)) return false;
        wf.time_resolution = static_cast<UChar_t>(tmp);
        if (!read_uint(tmp)) return false;
        wf.trigger_threshold = static_cast<UShort_t>(tmp);

        if (wf_size == 9) {
            if (!read_float64(wf.ns_per_sample)) return false;
        }
        return true;
    }

    const std::vector<uint8_t>& data_;
    size_t pos_;
    uint16_t energy_min_;
};

// ============================================================================
// File I/O
// ============================================================================

static bool read_file_info(const std::string& path, FileInfo& info) {
    std::ifstream f(path, std::ios::binary);
    if (!f.is_open()) return false;

    f.seekg(0, std::ios::end);
    auto file_size = f.tellg();
    if (file_size < static_cast<std::streamoff>(FOOTER_SIZE + 12))
        return false;

    f.seekg(0);
    char magic[8];
    f.read(magic, 8);
    if (std::memcmp(magic, FILE_MAGIC, 8) != 0) return false;

    f.seekg(file_size - static_cast<std::streamoff>(FOOTER_SIZE));
    char footer_magic[8];
    f.read(footer_magic, 8);
    if (std::memcmp(footer_magic, FOOTER_MAGIC, 8) != 0) return false;

    read_u64_le(f);  // checksum
    info.total_events = read_u64_le(f);
    read_u64_le(f);  // data_bytes
    info.first_event_time_ns = read_f64_le(f);
    info.last_event_time_ns = read_f64_le(f);

    info.path = path;
    return true;
}

/// Read all events from a .delila file into a vector (no sorting here)
/// Events with energy < energy_min are discarded at parse time.
static bool read_events_from_file(const std::string& path,
                                  std::vector<Event>& events,
                                  uint16_t energy_min = 0) {
    std::ifstream f(path, std::ios::binary);
    if (!f.is_open()) {
        std::cerr << "Error: Cannot open " << path << std::endl;
        return false;
    }

    f.seekg(0, std::ios::end);
    auto file_size = f.tellg();
    f.seekg(0);

    char magic[8];
    f.read(magic, 8);
    uint32_t header_len = read_u32_le(f);
    f.seekg(header_len, std::ios::cur);

    auto data_end = file_size - static_cast<std::streamoff>(FOOTER_SIZE);

    // Reusable block buffer to avoid per-block allocation
    std::vector<uint8_t> block_data;
    int block_count = 0;

    while (f.tellg() < data_end) {
        uint32_t block_len = read_u32_le(f);
        if (block_len == 0 || block_len > 100000000) break;
        if (f.tellg() + static_cast<std::streamoff>(block_len) > data_end)
            break;

        block_data.resize(block_len);
        f.read(reinterpret_cast<char*>(block_data.data()), block_len);
        if (!f.good()) break;

        MsgPackParser parser(block_data, energy_min);
        if (!parser.parse_batch(events)) {
            std::cerr << "Warning: Failed to parse block " << block_count
                      << " in " << path << std::endl;
            break;
        }
        block_count++;
    }
    return true;
}

// ============================================================================
// Scalar output — one event per TTree entry
// ============================================================================
struct ScalarBuffer {
    // Scalar fields
    UChar_t mod;
    UChar_t ch;
    UShort_t energy;
    UShort_t eshort;
    Double_t timestamp;
    ULong64_t flags;

    // Waveform fields
    std::vector<Short_t> analog_probe1;
    std::vector<Short_t> analog_probe2;
    std::vector<UChar_t> digital_probe1;
    std::vector<UChar_t> digital_probe2;
    std::vector<UChar_t> digital_probe3;
    std::vector<UChar_t> digital_probe4;
    UChar_t time_resolution = 0;
    UShort_t trigger_threshold = 0;
    Double_t ns_per_sample = 0.0;

    TTree* tree = nullptr;
    uint64_t total_written = 0;

    void add(Event& ev) {
        mod = ev.module;
        ch = ev.channel;
        energy = ev.energy;
        eshort = ev.energy_short;
        timestamp = ev.timestamp_ns;
        flags = ev.flags;

        if (ev.waveform) {
            // swap avoids extra allocation — buffer gets waveform's storage,
            // waveform gets buffer's (freed when Event is destroyed)
            analog_probe1.swap(ev.waveform->analog_probe1);
            analog_probe2.swap(ev.waveform->analog_probe2);
            digital_probe1.swap(ev.waveform->digital_probe1);
            digital_probe2.swap(ev.waveform->digital_probe2);
            digital_probe3.swap(ev.waveform->digital_probe3);
            digital_probe4.swap(ev.waveform->digital_probe4);
            time_resolution = ev.waveform->time_resolution;
            trigger_threshold = ev.waveform->trigger_threshold;
            ns_per_sample = ev.waveform->ns_per_sample;
        } else {
            analog_probe1.clear();
            analog_probe2.clear();
            digital_probe1.clear();
            digital_probe2.clear();
            digital_probe3.clear();
            digital_probe4.clear();
            time_resolution = 0;
            trigger_threshold = 0;
            ns_per_sample = 0.0;
        }

        tree->Fill();
        total_written++;
    }
};

// ============================================================================
// Two-pointer merge: carry_over × file_events → Fill + next carry_over
// ============================================================================
static void merge_and_flush(
    std::vector<Event>& carry_over,
    std::vector<Event>& file_events,
    double safe_threshold,
    ScalarBuffer& buf) {

    // Event is move-only (contains unique_ptr<WaveformData>), so we use
    // index-based iteration with std::move instead of const iterators.
    size_t ic = 0, if_ = 0;
    std::vector<Event> next_carry;

    while (ic < carry_over.size() || if_ < file_events.size()) {
        Event* pick;
        if (ic < carry_over.size() &&
            (if_ >= file_events.size() ||
             carry_over[ic].timestamp_ns <= file_events[if_].timestamp_ns)) {
            pick = &carry_over[ic++];
        } else {
            pick = &file_events[if_++];
        }

        if (pick->timestamp_ns < safe_threshold) {
            buf.add(*pick);
        } else {
            next_carry.push_back(std::move(*pick));
        }
    }

    carry_over = std::move(next_carry);
}

// ============================================================================
// Usage
// ============================================================================
static void print_usage(const char* prog) {
    std::cout << "Usage: " << prog
              << " -o <output.root> [--tree <name>] "
                 "<file1.delila> [file2.delila ...]"
              << std::endl;
    std::cout << "\nOptions:" << std::endl;
    std::cout << "  -o <file>         Output ROOT file (required)" << std::endl;
    std::cout << "  --tree <name>     TTree name (default: delila)" << std::endl;
    std::cout << "  --energy-min <n>  Discard events with energy < n (default: 0 = no filter)" << std::endl;
    std::cout << "  -h, --help        Show this help" << std::endl;
}

// ============================================================================
// Main
// ============================================================================
int main(int argc, char* argv[]) {
    ROOT::EnableImplicitMT();

    std::string output_file;
    std::string tree_name = "delila";
    uint16_t energy_min = 0;
    std::vector<std::string> input_files;

    for (int i = 1; i < argc; i++) {
        std::string arg = argv[i];
        if (arg == "-o" && i + 1 < argc) {
            output_file = argv[++i];
        } else if (arg == "--tree" && i + 1 < argc) {
            tree_name = argv[++i];
        } else if (arg == "--energy-min" && i + 1 < argc) {
            energy_min = static_cast<uint16_t>(std::stoi(argv[++i]));
        } else if (arg == "-h" || arg == "--help") {
            print_usage(argv[0]);
            return 0;
        } else if (arg[0] != '-') {
            input_files.push_back(arg);
        } else {
            std::cerr << "Unknown option: " << arg << std::endl;
            print_usage(argv[0]);
            return 1;
        }
    }

    if (output_file.empty() || input_files.empty()) {
        print_usage(argv[0]);
        return 1;
    }

    auto t_start = std::chrono::steady_clock::now();

    // ========================================================================
    // Phase 1: Read all footers and sort files by first_event_time
    // ========================================================================
    std::cout << "Reading footers from " << input_files.size() << " files..."
              << std::endl;

    std::vector<FileInfo> file_infos;
    uint64_t total_events_expected = 0;
    for (const auto& path : input_files) {
        FileInfo info;
        if (!read_file_info(path, info)) {
            std::cerr << "Warning: Skipping invalid file: " << path
                      << std::endl;
            continue;
        }
        total_events_expected += info.total_events;
        file_infos.push_back(info);
    }

    if (file_infos.empty()) {
        std::cerr << "Error: No valid .delila files found" << std::endl;
        return 1;
    }

    std::sort(file_infos.begin(), file_infos.end(),
              [](const FileInfo& a, const FileInfo& b) {
                  return a.first_event_time_ns < b.first_event_time_ns;
              });

    std::cout << "Files: " << file_infos.size()
              << ", Total events: " << total_events_expected << std::endl;
    std::cout << "Time range: " << file_infos.front().first_event_time_ns
              << " - " << file_infos.back().last_event_time_ns << " ns"
              << std::endl;
    std::cout << "Memory per file: ~"
              << (file_infos[0].total_events * sizeof(Event) / 1024 / 1024)
              << " MB (scalars only, " << sizeof(Event)
              << " bytes/event + waveform data on heap)" << std::endl;
    if (energy_min > 0) {
        std::cout << "Energy filter: energy >= " << energy_min << std::endl;
    }

    // ========================================================================
    // Phase 2: Set up ROOT output with LZ4 compression (scalar branches)
    // ========================================================================
    TFile* fout = new TFile(output_file.c_str(), "RECREATE");
    if (!fout || fout->IsZombie()) {
        std::cerr << "Error: Cannot create " << output_file << std::endl;
        return 1;
    }
    fout->SetCompressionAlgorithm(ROOT::RCompressionSetting::EAlgorithm::kLZ4);
    fout->SetCompressionLevel(1);

    TTree* tree = new TTree(tree_name.c_str(), "DELILA Data (time-sorted)");
    tree->SetAutoFlush(-100000000LL);  // 100MB byte-based flush

    ScalarBuffer buf;
    buf.tree = tree;

    tree->Branch("Mod", &buf.mod, "Mod/b");
    tree->Branch("Ch", &buf.ch, "Ch/b");
    tree->Branch("Energy", &buf.energy, "Energy/s");
    tree->Branch("EnergyShort", &buf.eshort, "EnergyShort/s");
    tree->Branch("Timestamp", &buf.timestamp, "Timestamp/D");
    tree->Branch("Flags", &buf.flags, "Flags/l");

    // Waveform branches (std::vector — ROOT has built-in dictionary)
    tree->Branch("AnalogProbe1", &buf.analog_probe1);
    tree->Branch("AnalogProbe2", &buf.analog_probe2);
    tree->Branch("DigitalProbe1", &buf.digital_probe1);
    tree->Branch("DigitalProbe2", &buf.digital_probe2);
    tree->Branch("DigitalProbe3", &buf.digital_probe3);
    tree->Branch("DigitalProbe4", &buf.digital_probe4);
    tree->Branch("TimeResolution", &buf.time_resolution, "TimeResolution/b");
    tree->Branch("TriggerThreshold", &buf.trigger_threshold, "TriggerThreshold/s");
    tree->Branch("NsPerSample", &buf.ns_per_sample, "NsPerSample/D");

    // ========================================================================
    // Phase 3: Per-file sort + two-pointer merge
    // ========================================================================
    std::vector<Event> carry_over;

    for (size_t fi = 0; fi < file_infos.size(); fi++) {
        const auto& finfo = file_infos[fi];
        std::cout << "\r[" << (fi + 1) << "/" << file_infos.size() << "] "
                  << finfo.path
                  << " (" << finfo.total_events << " events"
                  << ", carry=" << carry_over.size() << ")"
                  << "        " << std::flush;

        // (a) Read all events from this file
        std::vector<Event> file_events;
        file_events.reserve(finfo.total_events);
        if (!read_events_from_file(finfo.path, file_events, energy_min)) {
            std::cerr << "\nError reading " << finfo.path << std::endl;
            continue;
        }

        // (b) Sort this file's events by timestamp (move-only, sorts via pointer swap)
        std::sort(file_events.begin(), file_events.end(),
                  [](const Event& a, const Event& b) {
                      return a.timestamp_ns < b.timestamp_ns;
                  });

        // (c) Determine safe flush threshold
        double threshold;
        if (fi + 1 < file_infos.size()) {
            threshold = file_infos[fi + 1].first_event_time_ns;
        } else {
            threshold = std::numeric_limits<double>::max();
        }

        // (d) Two-pointer merge: carry_over × file_events → Fill
        merge_and_flush(carry_over, file_events, threshold, buf);
    }

    // Final flush: everything remaining in carry_over
    if (!carry_over.empty()) {
        std::cout << "\nFlushing remaining " << carry_over.size() << " events"
                  << std::endl;
        std::vector<Event> empty;
        merge_and_flush(
            carry_over, empty, std::numeric_limits<double>::max(), buf);
    }
    uint64_t total_written = buf.total_written;

    // ========================================================================
    // Phase 4: Write and close
    // ========================================================================
    std::cout << "\nWriting ROOT file..." << std::flush;
    tree->Write();
    // After ROOT auto-splits (file size > TTree::GetMaxTreeSize()), the
    // original fout is closed internally and tree moves to a new TFile.
    // Always use tree->GetCurrentFile() to close the active file.
    TFile* current_file = tree->GetCurrentFile();
    if (current_file) {
        current_file->Close();
    }

    auto t_end = std::chrono::steady_clock::now();
    double elapsed =
        std::chrono::duration<double>(t_end - t_start).count();

    std::cout << "\nDone." << std::endl;
    std::cout << "  Events: " << total_written << " / "
              << total_events_expected << " in files" << std::endl;
    if (energy_min > 0) {
        uint64_t filtered = total_events_expected - total_written;
        double pct = total_events_expected > 0
            ? filtered * 100.0 / total_events_expected : 0.0;
        std::cout << "  Filtered: " << filtered << " events removed"
                  << " (" << pct << "% below energy " << energy_min << ")"
                  << std::endl;
    }
    std::cout << "  Output: " << output_file << std::endl;
    std::cout << "  Time:   " << elapsed << " s" << std::endl;
    std::cout << "  Rate:   "
              << (total_written / 1e6 / elapsed) << " M events/s"
              << std::endl;

    if (energy_min == 0 && total_written != total_events_expected) {
        std::cerr << "WARNING: Event count mismatch!" << std::endl;
        return 1;
    }
    return 0;
}
