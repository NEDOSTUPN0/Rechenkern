// Times SoulverCore on the same lines and sheet as `calculate.rs`.
//
// Needs a SoulverCore-Multiplatform release (github.com/soulverteam/SoulverCore-Multiplatform)
// and a matching Swift toolchain:
//
//   swiftc -O soulver.swift -I <release> -L <release> -lSoulverCoreDynamic -o soulver-bench
//   LD_LIBRARY_PATH=<release> ./soulver-bench lines.txt sheet.txt
//
// The `SoulverCore_SoulverCore.resources` folder must sit next to the binary.

import Foundation
import SoulverCore

func now() -> UInt64 { DispatchTime.now().uptimeNanoseconds }
func micros(_ nanos: UInt64) -> Double { Double(nanos) / 1000 }

let args = CommandLine.arguments
let text = try! String(contentsOfFile: args[1], encoding: .utf8)
let lines = text.split(separator: "\n").map(String.init).filter { !$0.isEmpty && !$0.hasPrefix("//") }

let setupStart = now()
let calc = Calculator(customization: .standard)
print(String(format: "calculator setup:        %9.1f µs", micros(now() - setupStart)))

let firstStart = now()
for line in lines { _ = calc.calculate(line) }
print(String(format: "first pass:              %9.1f µs", micros(now() - firstStart)))

var times: [(Double, String)] = []
for line in lines {
    let start = now()
    var runs: UInt64 = 0
    while now() - start < 20_000_000 {
        for _ in 0..<10 { _ = calc.calculate(line) }
        runs += 10
    }
    times.append((micros((now() - start) / runs), line))
}
times.sort { $0.0 < $1.0 }
let mean = times.map { $0.0 }.reduce(0, +) / Double(times.count)
print(String(format: "lines:                   %9d", times.count))
print(String(format: "median:                  %9.2f µs", times[times.count / 2].0))
print(String(format: "mean:                    %9.2f µs", mean))
print(String(format: "lines per second:        %9.0f", 1_000_000 / mean))

// Variables need the customization the Soulver app uses.
let sheetText = try! String(contentsOfFile: args[2], encoding: .utf8)
let sheet = sheetText.split(separator: "\n", omittingEmptySubsequences: false).map(String.init).dropLast().map { $0 }
let collection = LineCollection(customization: .soulver)
var best = UInt64.max
for _ in 0..<20 {
    let start = now()
    collection.setLinesWithExpressions(sheet)
    collection.evaluateAll()
    best = min(best, now() - start)
}
print(String(format: "sheet of %d lines:       %9.1f µs", sheet.count, micros(best)))
