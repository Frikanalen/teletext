// Thin C ABI over the Blackmagic Desktop Video SDK for ttxd.
//
// Plays a constant black 720p50 picture by scheduled playback and attaches one ANC packet
// (the OP-47 SDP) to every frame through IDeckLinkVideoFrameAncillaryPackets. The SDK
// builds the SMPTE 291 packet (ADF, DID/SDID/DC, parity, checksum) from the user data words.
// Each completed frame calls back into Rust for the next frame's packet, so the output
// card is the frame clock.

#include "DeckLinkAPI.h"

#include <atomic>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <vector>

typedef size_t (*ttx_fill_fn)(void* user, uint8_t* buf, size_t cap);

namespace {

const int kWidth = 1280;
const int kHeight = 720;
const int kFrames = 6;  // frames in flight

class AncPacket : public IDeckLinkAncillaryPacket {
public:
    AncPacket(uint8_t did, uint8_t sdid, uint32_t line) : did_(did), sdid_(sdid), line_(line) {}
    void set(const uint8_t* data, size_t len) {
        std::lock_guard<std::mutex> g(m_);
        data_.assign(data, data + len);
    }
    HRESULT STDMETHODCALLTYPE GetBytes(BMDAncillaryPacketFormat format, const void** data, uint32_t* size) override {
        if (format != bmdAncillaryPacketFormatUInt8) return E_NOTIMPL;
        std::lock_guard<std::mutex> g(m_);
        if (data) *data = data_.data();
        if (size) *size = (uint32_t)data_.size();
        return S_OK;
    }
    uint8_t STDMETHODCALLTYPE GetDID() override { return did_; }
    uint8_t STDMETHODCALLTYPE GetSDID() override { return sdid_; }
    uint32_t STDMETHODCALLTYPE GetLineNumber() override { return line_; }
    uint8_t STDMETHODCALLTYPE GetDataStreamIndex() override { return 0; }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID, LPVOID* ppv) override {
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs_; }
    ULONG STDMETHODCALLTYPE Release() override {
        ULONG r = --refs_;
        if (r == 0) delete this;
        return r;
    }

private:
    std::atomic<ULONG> refs_{1};
    std::mutex m_;
    std::vector<uint8_t> data_;
    uint8_t did_, sdid_;
    uint32_t line_;
};

struct Slot {
    IDeckLinkMutableVideoFrame* frame = nullptr;
    IDeckLinkVideoFrameAncillaryPackets* anc = nullptr;
    AncPacket* packet = nullptr;
};

class Output : public IDeckLinkVideoOutputCallback {
public:
    IDeckLinkOutput* out = nullptr;
    BMDTimeValue duration = 0;
    BMDTimeScale scale = 0;
    BMDTimeValue next = 0;
    ttx_fill_fn fill = nullptr;
    void* user = nullptr;
    std::vector<Slot> slots;
    uint8_t buf[256];

    // Refill a frame's packet from Rust and schedule it.
    HRESULT schedule(Slot& s) {
        size_t n = fill(user, buf, sizeof buf);
        s.anc->DetachAllPackets();
        if (n > 0) {
            s.packet->set(buf, n);
            s.anc->AttachPacket(s.packet);
        }
        HRESULT r = out->ScheduleVideoFrame(s.frame, next, duration, scale);
        next += duration;
        return r;
    }

    HRESULT STDMETHODCALLTYPE ScheduledFrameCompleted(IDeckLinkVideoFrame* done, BMDOutputFrameCompletionResult) override {
        for (auto& s : slots)
            if (s.frame == done) return schedule(s);
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE ScheduledPlaybackHasStopped() override { return S_OK; }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID, LPVOID* ppv) override {
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return 1; }
    ULONG STDMETHODCALLTYPE Release() override { return 1; }
};

IDeckLink* find_device(const char* name) {
    IDeckLinkIterator* it = CreateDeckLinkIteratorInstance();
    if (!it) return nullptr;
    IDeckLink* dl = nullptr;
    while (it->Next(&dl) == S_OK) {
        const char* dn = nullptr;
        if (dl->GetDisplayName(&dn) == S_OK && dn) {
            bool match = std::strcmp(dn, name) == 0;
            std::free((void*)dn);
            if (match) break;
        }
        dl->Release();
        dl = nullptr;
    }
    it->Release();
    return dl;
}

}  // namespace

extern "C" {

// Start output on the device with this display name (e.g. "DeckLink Duo (2)").
// Returns 0 on success, or a negative step number on failure.
int ttx_decklink_start(const char* device, uint8_t did, uint8_t sdid, uint32_t line, ttx_fill_fn fill, void* user) {
    IDeckLink* dl = find_device(device);
    if (!dl) return -1;
    auto* o = new Output();
    o->fill = fill;
    o->user = user;
    if (dl->QueryInterface(IID_IDeckLinkOutput, (void**)&o->out) != S_OK) return -2;

    IDeckLinkDisplayMode* mode = nullptr;
    // 720p50, 8-bit YUV.
    if (o->out->GetDisplayMode(bmdModeHD720p50, &mode) != S_OK || !mode) return -3;
    mode->GetFrameRate(&o->duration, &o->scale);
    mode->Release();
    if (o->out->EnableVideoOutput(bmdModeHD720p50, bmdVideoOutputVANC) != S_OK) return -4;
    if (o->out->SetScheduledFrameCompletionCallback(o) != S_OK) return -5;

    for (int i = 0; i < kFrames; i++) {
        Slot s;
        if (o->out->CreateVideoFrame(kWidth, kHeight, kWidth * 2, bmdFormat8BitYUV, bmdFrameFlagDefault, &s.frame) != S_OK)
            return -6;
        void* px = nullptr;
        IDeckLinkVideoBuffer* vb = nullptr;
        // Black in UYVY: Cb/Cr 0x80, Y 0x10.
        if (s.frame->QueryInterface(IID_IDeckLinkVideoBuffer, (void**)&vb) == S_OK) {
            vb->StartAccess(bmdBufferAccessWrite);
            vb->GetBytes(&px);
        }
        if (px) {
            uint8_t* p = (uint8_t*)px;
            for (int j = 0; j < kWidth * kHeight * 2; j += 2) {
                p[j] = 0x80;
                p[j + 1] = 0x10;
            }
        }
        if (vb) {
            vb->EndAccess(bmdBufferAccessWrite);
            vb->Release();
        }
        if (s.frame->QueryInterface(IID_IDeckLinkVideoFrameAncillaryPackets, (void**)&s.anc) != S_OK) return -7;
        s.packet = new AncPacket(did, sdid, line);
        o->slots.push_back(s);
    }
    for (auto& s : o->slots)
        if (o->schedule(s) != S_OK) return -8;
    if (o->out->StartScheduledPlayback(0, o->scale, 1.0) != S_OK) return -9;
    return 0;
}

}  // extern "C"
