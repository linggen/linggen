// `ling-hear` — the helper `ling` runs to hear a clip with Apple's on-device recognition.
//
//   ling-hear state   <language>     where the language stands
//   ling-hear install <language>     put its model on this Mac (macOS 26 and later), then answer
//   ling-hear hear    <language>     the clip on stdin (raw 16 kHz mono 16-bit samples, to the end), then its words
//
// `<language>` is a language tag as a page writes it (en, zh-Hans), or "" for none named
// (`unnamedLanguage`). One JSON line on stdout: {"code": …} and, for `hear`, "text" and "language". The sound is read from stdin and
// held in memory; nothing is written to disk and nothing is sent anywhere.
// Built by build.rs, carried inside `ling` and run as its own process (`stt_apple.rs`): a failure here
// stops dictation, never the engine.

import Foundation

func answer(_ code: String, text: String? = nil, language: String? = nil) -> Never {
    var body: [String: String] = ["code": code]
    if let text { body["text"] = text }
    if let language { body["language"] = language }
    let line = try! JSONSerialization.data(withJSONObject: body, options: [.sortedKeys])
    FileHandle.standardOutput.write(line + Data("\n".utf8))
    exit(0)
}

let args = CommandLine.arguments
guard args.count == 3 else { answer(Code.failed) }
let verb = args[1]
let tag = args[2].isEmpty ? unnamedLanguage() : args[2]
switch verb {
case "state": answer(stateOf(tag))
case "install": answer(install(tag))
case "hear":
    let pcm = FileHandle.standardInput.readDataToEndOfFile()
    let samples = pcm.withUnsafeBytes { Array($0.bindMemory(to: Int16.self)) }
    let heard = hear(tag, samples)
    answer(heard.code, text: heard.text, language: Locale(identifier: tag).languageCode)
default: answer(Code.failed)
}
