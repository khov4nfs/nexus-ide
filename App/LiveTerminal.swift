#if NEXUS_UBUNTU_LAB && os(iOS) && !targetEnvironment(simulator)
import SwiftUI
import UIKit
import Foundation
import SwiftTerm

/// A local PTY front end. No SSH transport or host command execution.
struct LiveTerminalScreen: View {
    @Environment(\.dismiss) private var dismiss
    @State private var status = "Connecting to local PTY"
    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Text(status).font(.caption).padding(6)
                LiveTerminal(status: $status)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .navigationTitle("Linux terminal")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close view") { dismiss() } } }
        }
    }
}

private struct LiveTerminal: UIViewRepresentable {
    @Binding var status: String
    func makeCoordinator() -> Coordinator { Coordinator(status: $status) }
    func makeUIView(context: Context) -> TerminalView {
        let terminal = TerminalView(frame: .zero)
        terminal.terminalDelegate = context.coordinator
        context.coordinator.start(terminal)
        return terminal
    }
    func updateUIView(_ view: TerminalView, context: Context) {}
    static func dismantleUIView(_ view: TerminalView, coordinator: Coordinator) {
        coordinator.stop()
        view.terminalDelegate = nil
    }
    final class Coordinator: NSObject, TerminalViewDelegate {
        @Binding var status: String
        private var timer: Timer?
        private var cursor: UInt64 = 0
        private var ready = false
        private var failed = false
        private var pendingSize: (Int,Int)?
        init(status: Binding<String>) { _status = status }
        private func decode(_ pointer: UnsafeMutablePointer<CChar>?) -> [String:Any] {
            guard let pointer else { return [:] }
            defer { nexus_guest_free(pointer) }
            return (try? JSONSerialization.jsonObject(with: Data(String(cString: pointer).utf8)) as? [String:Any]) ?? [:]
        }
        func start(_ terminal: TerminalView) {
            timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self,weak terminal] _ in
                guard let self,let terminal else {return}
                self.poll(terminal)
            }
        }
        func stop() {timer?.invalidate();timer=nil;ready=false}
        private func poll(_ view: TerminalView) {
            guard !failed else {return}
            let report=decode(nexus_local_terminal_delta(cursor))
            guard report["ok"] as? Bool == true,
                  let next=report["next"] as? UInt64, next>=cursor,
                  let encoded=report["bytes"] as? String,
                  let bytes=Data(base64Encoded: encoded), UInt64(bytes.count)==next-cursor else {
                failed=true;ready=false
                status="PTY history unavailable or lost. Close this view and use plain output; no input sent."
                return
            }
            ready=report["ready"] as? Bool == true
            if ready, let size=pendingSize {
                if decode(nexus_local_resize(UInt16(size.0),UInt16(size.1)))["ok"] as? Bool == true {pendingSize=nil}
            }
            cursor=next
            if !bytes.isEmpty {view.feed(byteArray: Array(bytes)[...])}
            status=ready ? "Local PTY · interactive" : "Process not accepting input"
        }
        func send(source: TerminalView,data: ArraySlice<UInt8>) {
            guard ready,!failed else {return}
            guard !data.isEmpty,data.count<=1024 else {status="Input exceeds 1024 bytes; paste smaller chunks";return}
            let encoded=Data(data).base64EncodedString()
            if encoded.withCString({decode(nexus_local_terminal_input($0))["ok"] as? Bool == true}) != true {
                status="PTY input not accepted"
            }
        }
        func sizeChanged(source: TerminalView,newCols:Int,newRows:Int) {
            guard (20...240).contains(newCols),(5...100).contains(newRows) else {return}
            pendingSize=(newCols,newRows)
        }
        func setTerminalTitle(source: TerminalView,title:String) {}
        func hostCurrentDirectoryUpdate(source:TerminalView,directory:String?) {}
        func scrolled(source:TerminalView,position:Double) {}
        func requestOpenLink(source:TerminalView,link:String,params:[String:String]) {}
        func bell(source:TerminalView) {}
        func clipboardCopy(source:TerminalView,content:Data) {}
        func clipboardRead(source:TerminalView)->Data? {nil}
        func iTermContent(source:TerminalView,content:ArraySlice<UInt8>) {}
        func rangeChanged(source:TerminalView,startY:Int,endY:Int) {}
    }
}
#endif
