#if NEXUS_FULL_VM && NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import Foundation
import UniformTypeIdentifiers
import CryptoKit
import Darwin
import UIKit

// The interpreter is linked as a signed app-bundled framework, not an
// executable imported by the user. Guest files are never native executables.
private enum VMAsset: String, CaseIterable, Identifiable {
    case kernel, initrd, disk
    var id: String { rawValue }
    var label: String {
        switch self {
        case .kernel: return "ARM64 Linux Image"
        case .initrd: return "Linux initramfs"
        case .disk: return "Raw ext4 root disk"
        }
    }
    var limit: UInt64 {
        switch self {
        case .kernel: return 128 * 1024 * 1024
        case .initrd: return 512 * 1024 * 1024
        case .disk: return 8 * 1024 * 1024 * 1024
        }
    }
}

private enum VMFailure: Error { case invalid, unavailable, io }

private enum VMStore {
    static func directory() throws -> URL {
        let fm = FileManager.default
        var url = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("NexusFullVM-v1", isDirectory: true)
        try fm.createDirectory(at: url, withIntermediateDirectories: true,
                               attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        var attributes = URLResourceValues()
        attributes.isExcludedFromBackup = true
        try url.setResourceValues(attributes)
        return url
    }
    static func file(_ root: URL, _ kind: VMAsset) -> URL { root.appendingPathComponent(kind.rawValue) }
    static func proof(_ root: URL, _ kind: VMAsset) -> URL { root.appendingPathComponent(kind.rawValue + ".sha256") }
    static func dirty(_ root: URL) -> URL { root.appendingPathComponent("boot-in-progress") }
    static func validDigest(_ s: String) -> Bool {
        s.utf8.count == 64 && s.utf8.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
    }
    static func regularSize(_ url: URL, limit: UInt64) throws -> UInt64 {
        let value = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard value.isRegularFile == true, value.isSymbolicLink != true, let bytes = value.fileSize,
              bytes > 0, UInt64(bytes) <= limit else { throw VMFailure.invalid }
        return UInt64(bytes)
    }
    static func digest(_ url: URL, limit: UInt64) throws -> String {
        let expected = try regularSize(url, limit: limit)
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var sha = SHA256()
        var total: UInt64 = 0
        while let bytes = try handle.read(upToCount: 1024 * 1024), !bytes.isEmpty {
            total += UInt64(bytes.count)
            guard total <= expected else { throw VMFailure.invalid }
            sha.update(data: bytes)
        }
        guard total == expected else { throw VMFailure.invalid }
        return sha.finalize().map { String(format: "%02x", $0) }.joined()
    }
    static func validateFormat(_ url: URL, _ kind: VMAsset) throws {
        let fd = try FileHandle(forReadingFrom: url)
        defer { try? fd.close() }
        switch kind {
        case .kernel:
            try fd.seek(toOffset: 56)
            guard try fd.read(upToCount: 4) == Data([0x41, 0x52, 0x4d, 0x64]) else { throw VMFailure.invalid }
        case .initrd:
            // Kernel performs actual initramfs decompression/validation.
            _ = try regularSize(url, limit: kind.limit)
        case .disk:
            // A raw ext4 filesystem, not qcow, sparse image or partitioned disk.
            try fd.seek(toOffset: 1080)
            guard try fd.read(upToCount: 2) == Data([0x53, 0xef]) else { throw VMFailure.invalid }
        }
    }
    static func importAsset(_ source: URL, kind: VMAsset, expected: String) throws {
        guard validDigest(expected) else { throw VMFailure.invalid }
        let root = try directory()
        let destination = file(root, kind)
        let marker = proof(root, kind)
        let fm = FileManager.default
        guard !fm.fileExists(atPath: dirty(root).path),
              !fm.fileExists(atPath: destination.path), !fm.fileExists(atPath: marker.path) else { throw VMFailure.invalid }
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let size = try regularSize(source, limit: kind.limit)
        let staging = root.appendingPathComponent(UUID().uuidString + ".partial")
        defer { try? fm.removeItem(at: staging) }
        guard fm.createFile(atPath: staging.path, contents: nil, attributes: [
            .protectionKey: FileProtectionType.completeUntilFirstUserAuthentication,
            .posixPermissions: 0o600
        ]) else { throw VMFailure.io }
        let input = try FileHandle(forReadingFrom: source)
        defer { try? input.close() }
        let output = try FileHandle(forWritingTo: staging)
        defer { try? output.close() }
        var hash = SHA256()
        var total: UInt64 = 0
        while let bytes = try input.read(upToCount: 1024 * 1024), !bytes.isEmpty {
            total += UInt64(bytes.count)
            guard total <= size else { throw VMFailure.invalid }
            hash.update(data: bytes)
            try output.write(contentsOf: bytes)
        }
        try output.synchronize()
        try output.close()
        let actual = hash.finalize().map { String(format: "%02x", $0) }.joined()
        guard total == size, actual == expected else { throw VMFailure.invalid }
        try validateFormat(staging, kind)
        try fm.moveItem(at: staging, to: destination)
        do {
            try Data((expected + "\n").utf8).write(to: marker, options: .atomic)
        } catch {
            try? fm.removeItem(at: destination)
            throw error
        }
    }
    static func ready(_ root: URL) -> Bool {
        guard !FileManager.default.fileExists(atPath: dirty(root).path) else { return false }
        return VMAsset.allCases.allSatisfy { kind in
            guard (try? regularSize(file(root, kind), limit: kind.limit)) != nil,
                  let s = try? String(contentsOf: proof(root, kind), encoding: .utf8) else { return false }
            return validDigest(s.trimmingCharacters(in: .whitespacesAndNewlines))
        }
    }
    static func verifyBoot(_ root: URL) throws {
        guard ready(root) else { throw VMFailure.invalid }
        for kind in VMAsset.allCases {
            try validateFormat(file(root, kind), kind)
            // Mutable root disk cannot retain its original hash after boot.
            // Immutable kernel and initrd are rehashed on every launch.
            if kind != .disk {
                let expected = try String(contentsOf: proof(root, kind), encoding: .utf8)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                guard try digest(file(root, kind), limit: kind.limit) == expected else { throw VMFailure.invalid }
            }
        }
    }
    static func reset() throws {
        try FileManager.default.removeItem(at: directory())
    }
}

private enum VMEngine {
    typealias Entry = @convention(c) (Int32, UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?) -> Int32
    // Deliberately never dlclose: QEMU has process-global state, threads and
    // registered exit notifiers. The app permits only one boot per process.
    static func load() throws -> Entry {
        guard let directory = Bundle.main.privateFrameworksURL else { throw VMFailure.unavailable }
        let binary = directory.appendingPathComponent("qemu-aarch64-softmmu.framework/qemu-aarch64-softmmu")
        guard let library = dlopen(binary.path, RTLD_NOW | RTLD_LOCAL),
              let symbol = dlsym(library, "nexus_qemu_tci_run") else { throw VMFailure.unavailable }
        return unsafeBitCast(symbol, to: Entry.self)
    }
    static func invoke(_ entry: Entry, arguments: [String]) -> Int32 {
        var argv = arguments.map { strdup($0) }
        guard argv.allSatisfy({ $0 != nil }) else {
            argv.forEach { free($0) }
            return -1
        }
        argv.append(nil)
        defer { argv.forEach { free($0) } }
        return argv.withUnsafeMutableBufferPointer { entry(Int32(arguments.count), $0.baseAddress) }
    }
    static func socketPair() throws -> (Int32, Int32) {
        var pair: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw VMFailure.io }
        var yes: Int32 = 1
        guard setsockopt(pair[0], SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size)) == 0,
              fcntl(pair[0], F_SETFL, O_NONBLOCK) == 0 else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); throw VMFailure.io
        }
        return (pair[0], pair[1])
    }
    static func args(root: URL, firmware: URL, ram: Int, serial: Int32, monitor: Int32) throws -> [String] {
        let file: [String: Any] = ["driver": "file", "filename": VMStore.file(root, .disk).path, "node-name": "rootfile"]
        let raw: [String: Any] = ["driver": "raw", "file": "rootfile", "node-name": "rootdisk"]
        func encode(_ value: [String: Any]) throws -> String {
            String(decoding: try JSONSerialization.data(withJSONObject: value), as: UTF8.self)
        }
        return ["qemu-system-aarch64", "-L", firmware.path,
                "-machine", "virt,gic-version=3", "-cpu", "cortex-a57",
                "-accel", "tcg,thread=single", "-smp", "1", "-m", String(ram),
                "-nodefaults", "-no-user-config", "-display", "none", "-nic", "none", "-no-reboot",
                "-kernel", VMStore.file(root, .kernel).path, "-initrd", VMStore.file(root, .initrd).path,
                "-append", "root=/dev/vda rw rootwait console=ttyAMA0",
                "-blockdev", try encode(file), "-blockdev", try encode(raw),
                "-device", "virtio-blk-device,drive=rootdisk",
                "-chardev", "socket,id=serial0,fd=\(serial)", "-serial", "chardev:serial0",
                "-chardev", "socket,id=qmp0,fd=\(monitor)", "-mon", "chardev=qmp0,mode=control"]
    }
}

@MainActor private final class FullVMModel: ObservableObject {
    @Published var status = "Full-system VM candidate — not device verified."
    @Published var console = ""
    @Published var active = false
    @Published var ready = false
    @Published var busy = false
    @Published var monitorReady = false
    @Published var assets: Set<VMAsset> = []
    private var root: URL?
    private var serial: Int32 = -1
    private var monitor: Int32 = -1
    private var monitorBuffer = Data()
    private var serialTail = Data()
    private var timer: Timer?
    private var observers: [NSObjectProtocol] = []
    private var guestShutdown = false
    private var forced = false
    private var greeting = false
    private var powerRequested = false
    private var consumed = false
    private var deadline: Date?

    func refresh() {
        guard let root = try? VMStore.directory() else { ready = false; return }
        self.root = root
        assets = Set(VMAsset.allCases.filter {
            (try? VMStore.regularSize(VMStore.file(root, $0), limit: $0.limit)) != nil
        })
        ready = VMStore.ready(root)
        if FileManager.default.fileExists(atPath: VMStore.dirty(root).path), !active {
            status = "Previous boot did not prove a clean shutdown. Reset and re-import before reuse."
        }
    }
    func importFile(_ url: URL, kind: VMAsset, digest: String) {
        guard !busy, !active, !consumed else { return }
        busy = true
        status = "Copying and verifying selected guest asset…"
        Task {
            let result = await Task.detached(priority: .utility) { () -> Bool in
                do { try VMStore.importAsset(url, kind: kind, expected: digest); return true }
                catch { return false }
            }.value
            busy = false
            status = result ? "Reviewed guest asset imported." : "Import failed: format, size, digest, storage or existing asset."
            refresh()
        }
    }
    func reset() {
        guard !active, !busy, !consumed else { return }
        do { try VMStore.reset(); status = "Private VM assets removed."; refresh() }
        catch { status = "Reset failed." }
    }
    func start(ram: Int, claim: () -> Bool) {
        guard !busy, !active, !consumed, ready, let root, [512, 768, 1024].contains(ram),
              let firmware = Bundle.main.resourceURL?.appendingPathComponent("FullVMFirmware"),
              FileManager.default.fileExists(atPath: firmware.path),
              let entry = try? VMEngine.load(), claim() else {
            status = "VM unavailable, guest already claimed, or assets incomplete."; return
        }
        consumed = true
        busy = true
        status = "Verifying immutable boot assets…"
        Task {
            let valid = await Task.detached(priority: .utility) { () -> Bool in
                do { try VMStore.verifyBoot(root); return true } catch { return false }
            }.value
            guard valid, UIApplication.shared.applicationState == .active else {
                busy = false; status = "Boot verification failed or app is not foreground; relaunch to retry."; return
            }
            var serialPair: (Int32, Int32)?
            var monitorPair: (Int32, Int32)?
            do {
                let s = try VMEngine.socketPair(); serialPair = s
                let m = try VMEngine.socketPair(); monitorPair = m
                let args = try VMEngine.args(root: root, firmware: firmware, ram: ram, serial: s.1, monitor: m.1)
                try Data("dirty\n".utf8).write(to: VMStore.dirty(root), options: .atomic)
                serial = s.0; monitor = m.0
                active = true; busy = false; ready = false
                deadline = Date().addingTimeInterval(1800)
                status = "Booting local interpreter VM. Keep this app foreground."
                let pollTimer = Timer(timeInterval: 0.1, repeats: true) { [weak self] _ in
                    Task { @MainActor in self?.poll() }
                }
                timer = pollTimer
                RunLoop.main.add(pollTimer, forMode: .common)
                // Stop requests must work even when another app tab is selected.
                for notification in [UIApplication.willResignActiveNotification, UIApplication.didReceiveMemoryWarningNotification] {
                    observers.append(NotificationCenter.default.addObserver(forName: notification, object: nil, queue: .main) { [weak self] _ in
                        Task { @MainActor in self?.foregroundLost() }
                    })
                }
                // Ownership of the guest-side descriptors transfers to QEMU.
                // Do not close them again from Swift.
                DispatchQueue.global(qos: .userInitiated).async {
                    let code = VMEngine.invoke(entry, arguments: args)
                    Task { @MainActor in self.ended(code, root: root) }
                }
            } catch {
                if let pair = serialPair { Darwin.close(pair.0); Darwin.close(pair.1) }
                if let pair = monitorPair { Darwin.close(pair.0); Darwin.close(pair.1) }
                busy = false; status = "Unable to establish private VM transport."
            }
        }
    }
    private func readSocket(_ fd: Int32) -> Data {
        guard fd >= 0 else { return Data() }
        var all = Data()
        var bytes = [UInt8](repeating: 0, count: 8192)
        for _ in 0..<8 {
            let count = bytes.withUnsafeMutableBytes { Darwin.recv(fd, $0.baseAddress, $0.count, 0) }
            if count <= 0 { break }
            all.append(contentsOf: bytes.prefix(count))
        }
        return all
    }
    private func writeSocket(_ fd: Int32, _ data: Data) -> Bool {
        guard fd >= 0, !data.isEmpty, data.count <= 2048 else { return false }
        let count = data.withUnsafeBytes { Darwin.send(fd, $0.baseAddress, $0.count, 0) }
        // A partially delivered frame cannot safely be retried as a whole.
        if count != data.count {
            forced = true
            status = "Transport write incomplete. Disk will remain dirty; do not retry the command."
            return false
        }
        return true
    }
    private func qmp(_ command: String, id: String) -> Bool {
        guard let data = try? JSONSerialization.data(withJSONObject: ["execute": command, "id": id]) else { return false }
        return writeSocket(monitor, data + Data("\r\n".utf8))
    }
    private func poll() {
        guard active else { return }
        serialTail.append(readSocket(serial))
        if serialTail.count > 65536 { serialTail = Data(serialTail.suffix(65536)) }
        console = String(decoding: serialTail, as: UTF8.self)
        monitorBuffer.append(readSocket(monitor))
        if monitorBuffer.count > 65536 {
            monitorBuffer.removeAll(); forced = true
            status = "QMP frame limit exceeded. Disk will remain dirty."
        }
        while let index = monitorBuffer.firstIndex(of: 10) {
            let line = Data(monitorBuffer[..<index])
            monitorBuffer.removeSubrange(...index)
            guard let obj = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any] else {
                forced = true; continue
            }
            if obj["QMP"] != nil, !greeting {
                greeting = true
                _ = qmp("qmp_capabilities", id: "capabilities")
            }
            if obj["id"] as? String == "capabilities", obj["return"] != nil {
                monitorReady = true
                status = "QMP connected. Verify Linux boot in the serial console."
                if powerRequested { _ = qmp("system_powerdown", id: "powerdown") }
            }
            if obj["event"] as? String == "SHUTDOWN", let data = obj["data"] as? [String: Any],
               data["guest"] as? Bool == true, data["reason"] as? String == "guest-shutdown" {
                guestShutdown = true
            }
            if obj["error"] != nil { status = "QMP rejected a control request." }
        }
        let thermal = ProcessInfo.processInfo.thermalState
        if thermal == .serious || thermal == .critical { powerdown() }
        if let deadline, Date() >= deadline, !powerRequested { powerdown() }
    }
    func send(_ text: String) -> Bool {
        guard active, monitorReady, !forced, !powerRequested, !text.isEmpty,
              text.utf8.count <= 1024, !text.utf8.contains(0) else { return false }
        return writeSocket(serial, Data((text + "\r").utf8))
    }
    func interrupt() { if active && monitorReady && !forced && !powerRequested { _ = writeSocket(serial, Data([3])) } }
    func powerdown() {
        guard active, !powerRequested else { return }
        powerRequested = true
        status = "Requested guest powerdown. Wait for guest shutdown; use Force stop only if necessary."
        if monitorReady { _ = qmp("system_powerdown", id: "powerdown") }
    }
    func forceStop() {
        guard active, monitorReady else { return }
        forced = true
        status = "Forced stop requested. The disk will require reset."
        _ = qmp("quit", id: "quit")
    }
    func foregroundLost() { if active { powerdown() } }
    private func ended(_ code: Int32, root: URL) {
        poll()
        timer?.invalidate(); timer = nil
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers.removeAll()
        Darwin.close(serial); Darwin.close(monitor)
        serial = -1; monitor = -1; active = false; monitorReady = false
        if code == 0, guestShutdown, !forced {
            do {
                try FileManager.default.removeItem(at: VMStore.dirty(root))
                status = "Guest shutdown and QEMU cleanup observed. Reopen app to boot saved disk."
            } catch { status = "Shutdown returned but dirty marker could not be cleared." }
        } else {
            status = "VM stopped without proven guest shutdown. Disk remains dirty."
        }
        ready = false
    }
}

@MainActor struct FullVMView: View {
    @ObservedObject var model: IDEModel
    @StateObject private var vm = FullVMModel()
    @Environment(\.scenePhase) private var scenePhase
    @State private var selected = VMAsset.kernel
    @State private var expected = ""
    @State private var frozenKind = VMAsset.kernel
    @State private var frozenDigest = ""
    @State private var picking = false
    @State private var confirmStart = false
    @State private var confirmReset = false
    @State private var confirmForce = false
    @State private var ram = 768
    @State private var command = ""
    var body: some View {
        NavigationStack {
            List {
                Section("Local ARM64 full-system VM") {
                    Text(vm.status)
                    Text("Experimental QEMU TCI. No JIT, network, shared native directory or remote backend. Import only trusted compatible Linux boot assets. A fatal QEMU error may terminate the app; memory and thermal limits are not yet qualified.")
                        .font(.caption)
                    Picker("Asset", selection: $selected) {
                        ForEach(VMAsset.allCases) { asset in Text(asset.label).tag(asset) }
                    }.disabled(vm.busy || model.guestClaimed)
                    TextField("Reviewed SHA-256", text: $expected)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .disabled(vm.busy || model.guestClaimed)
                    Button("Import reviewed asset") {
                        frozenKind = selected; frozenDigest = expected; picking = true
                    }.disabled(vm.busy || model.guestClaimed || vm.assets.contains(selected) || !VMStore.validDigest(expected))
                    ForEach(VMAsset.allCases) { asset in
                        Label(asset.label, systemImage: vm.assets.contains(asset) ? "checkmark.circle" : "circle")
                    }
                    Button("Reset private VM disk and boot assets", role: .destructive) { confirmReset = true }
                        .disabled(vm.busy || model.guestClaimed)
                    Picker("Guest memory (MiB)", selection: $ram) {
                        Text("512").tag(512); Text("768").tag(768); Text("1024").tag(1024)
                    }.disabled(vm.busy || model.guestClaimed)
                    Button("Boot reviewed VM") { confirmStart = true }
                        .disabled(!vm.ready || vm.busy || model.guestClaimed)
                    if vm.busy { ProgressView() }
                }
                Section("Serial console") {
                    ScrollView {
                        Text(verbatim: vm.console.isEmpty ? "No guest output." : vm.console)
                            .font(.system(.caption, design: .monospaced))
                            .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                    }.frame(height: 260)
                    TextField("Guest serial command", text: $command)
                        .autocorrectionDisabled().textInputAutocapitalization(.never)
                        .disabled(!vm.monitorReady)
                    Button("Send to serial") { if vm.send(command) { command = "" } }
                        .disabled(!vm.monitorReady || command.isEmpty)
                    Button("Ctrl-C") { vm.interrupt() }.disabled(!vm.monitorReady)
                    Button("Request guest shutdown") { vm.powerdown() }.disabled(!vm.active)
                    Button("Force stop (dirty disk)", role: .destructive) { confirmForce = true }
                        .disabled(!vm.monitorReady)
                }
                Section("Container scope") {
                    Text("A compatible guest image must contain its own Docker Engine and Compose. No daemon or image is bundled. Use the serial console to test the guest and run sync; poweroff for clean shutdown if ACPI powerdown is unsupported. Host CI evidence is not iPhone evidence.")
                        .font(.caption)
                }
            }.navigationTitle("Full VM")
        }
        .onAppear { vm.refresh() }
        .onChange(of: scenePhase) { _, phase in if phase != .active { vm.foregroundLost() } }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.data]) { result in
            if case .success(let url) = result { vm.importFile(url, kind: frozenKind, digest: frozenDigest) }
        }
        .confirmationDialog("Boot trusted Linux image on this device?", isPresented: $confirmStart, titleVisibility: .visible) {
            Button("Boot offline interpreter VM") { vm.start(ram: ram) { model.claimGuest() } }
        }
        .confirmationDialog("Delete all private VM boot assets and disk?", isPresented: $confirmReset, titleVisibility: .visible) {
            Button("Delete VM files", role: .destructive) { vm.reset() }
        }
        .confirmationDialog("Force stop? The disk cannot be reused until reset.", isPresented: $confirmForce, titleVisibility: .visible) {
            Button("Force stop", role: .destructive) { vm.forceStop() }
        }
    }
}
#endif
