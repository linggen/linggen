// Apple's on-device speech recognition: the recognition itself, for `main.swift`.
//
// Everything here is blocking. The sound arrives as samples (16 kHz mono 16-bit)
// and is recognised on this Mac only:
//   - macOS 26 and later: SpeechTranscriber (SpeechAnalyzer), which always runs
//     on the device; a missing language model is put on this Mac by `install`, never by `hear`.
//   - before 26: SFSpeechRecognizer with requiresOnDeviceRecognition = true. The
//     server is never used; a language with no on-device model is `needsSettings`.
// The words of `Code` are mirrored in `stt_apple.rs` (`Verdict`); change both together.

import AVFoundation
import Foundation
import Speech

enum Code {
    static let ready = "ready"
    static let needsDownload = "needs_download"
    static let downloading = "downloading"
    static let unsupported = "unsupported"
    static let notAllowed = "not_allowed"
    static let failed = "failed"
    static let needsSettings = "needs_settings"
}

/// Run `body` to its end and return what it returns, from synchronous code.
private func blocking<T>(_ body: @escaping @Sendable () async -> T) -> T {
    let done = DispatchSemaphore(value: 0)
    let box = Box<T>()
    Task.detached {
        box.value = await body()
        done.signal()
    }
    done.wait()
    return box.value!
}

private final class Box<T>: @unchecked Sendable { var value: T? }

private func requested(_ tag: String) -> Locale { Locale(identifier: tag) }

// MARK: - macOS 26 and later

@available(macOS 26.0, *)
private func transcriber(for locale: Locale) async -> SpeechTranscriber? {
    guard SpeechTranscriber.isAvailable,
          let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) else { return nil }
    return SpeechTranscriber(locale: supported, transcriptionOptions: [], reportingOptions: [], attributeOptions: [])
}

@available(macOS 26.0, *)
private func stateModern(_ locale: Locale) async -> String {
    guard let module = await transcriber(for: locale) else { return Code.unsupported }
    // The installed list is what says a language is on this Mac; the inventory's own status
    // reads "supported" for an installed language that this program has not reserved.
    let wanted = module.selectedLocales.map { $0.identifier(.bcp47) }
    if await SpeechTranscriber.installedLocales.contains(where: { wanted.contains($0.identifier(.bcp47)) }) { return Code.ready }
    return await AssetInventory.status(forModules: [module]) == .downloading ? Code.downloading : Code.needsDownload
}

@available(macOS 26.0, *)
private func installModern(_ locale: Locale) async -> String {
    guard let module = await transcriber(for: locale) else { return Code.unsupported }
    do {
        // nil: nothing to install.
        try await AssetInventory.assetInstallationRequest(supporting: [module])?.downloadAndInstall()
        return Code.ready
    } catch {
        return Code.failed
    }
}

private func buffer(_ samples: [Int16]) -> AVAudioPCMBuffer? {
    guard let format = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: 16_000, channels: 1, interleaved: true),
          let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)) else { return nil }
    buffer.frameLength = AVAudioFrameCount(samples.count)
    samples.withUnsafeBufferPointer { buffer.int16ChannelData![0].update(from: $0.baseAddress!, count: samples.count) }
    return buffer
}

/// `source` converted to `format`, or itself when it is already in it.
private func converted(_ source: AVAudioPCMBuffer, to format: AVAudioFormat) -> AVAudioPCMBuffer? {
    if source.format == format { return source }
    guard let converter = AVAudioConverter(from: source.format, to: format) else { return nil }
    let ratio = format.sampleRate / source.format.sampleRate
    guard let out = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(Double(source.frameLength) * ratio) + 1024) else { return nil }
    var fed = false
    var failure: NSError?
    converter.convert(to: out, error: &failure) { _, status in
        if fed { status.pointee = .endOfStream; return nil }
        fed = true
        status.pointee = .haveData
        return source
    }
    return failure == nil ? out : nil
}

@available(macOS 26.0, *)
private func hearModern(_ locale: Locale, _ samples: [Int16]) async -> (code: String, text: String) {
    let state = await stateModern(locale)
    guard state == Code.ready else { return (state, "") }
    guard let module = await transcriber(for: locale),
          let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [module]),
          let heard = buffer(samples).flatMap({ converted($0, to: format) }) else { return (Code.failed, "") }
    let analyzer = SpeechAnalyzer(modules: [module])
    let collect = Task { () -> String in
        var words = ""
        do { for try await result in module.results { words += String(result.text.characters) } } catch {}
        return words
    }
    let (input, feed) = AsyncStream<AnalyzerInput>.makeStream()
    do {
        try await analyzer.start(inputSequence: input)
        feed.yield(AnalyzerInput(buffer: heard))
        feed.finish()
        try await analyzer.finalizeAndFinishThroughEndOfInput()
    } catch {
        collect.cancel()
        return (Code.failed, "")
    }
    return (Code.ready, await collect.value)
}

// MARK: - before macOS 26

/// The on-device recognizer for the language, when this Mac has one.
private func legacyRecognizer(_ locale: Locale) -> SFSpeechRecognizer? {
    let code = locale.languageCode
    let match = SFSpeechRecognizer.supportedLocales().first { $0.identifier == locale.identifier }
        ?? SFSpeechRecognizer.supportedLocales().first { $0.languageCode == code }
    return match.flatMap { SFSpeechRecognizer(locale: $0) }
}

private func stateLegacy(_ locale: Locale) -> String {
    guard let recognizer = legacyRecognizer(locale) else { return Code.unsupported }
    return recognizer.supportsOnDeviceRecognition ? Code.ready : Code.needsSettings
}

/// Speech recognition is allowed for this program, asking once if nobody was asked.
/// A process whose Info.plist lacks the usage text is killed by macOS when it asks,
/// so it never asks.
private func allowedLegacy() -> Bool {
    switch SFSpeechRecognizer.authorizationStatus() {
    case .authorized: return true
    case .denied, .restricted: return false
    case .notDetermined:
        guard Bundle.main.object(forInfoDictionaryKey: "NSSpeechRecognitionUsageDescription") != nil else { return false }
        let done = DispatchSemaphore(value: 0)
        let answer = Box<Bool>()
        SFSpeechRecognizer.requestAuthorization { answer.value = $0 == .authorized; done.signal() }
        done.wait()
        return answer.value ?? false
    @unknown default: return false
    }
}

private func hearLegacy(_ locale: Locale, _ samples: [Int16]) -> (code: String, text: String) {
    let state = stateLegacy(locale)
    guard state == Code.ready, let recognizer = legacyRecognizer(locale), let heard = buffer(samples) else { return (state == Code.ready ? Code.failed : state, "") }
    guard allowedLegacy() else { return (Code.notAllowed, "") }
    let request = SFSpeechAudioBufferRecognitionRequest()
    request.requiresOnDeviceRecognition = true
    request.shouldReportPartialResults = false
    request.append(heard)
    request.endAudio()
    let done = DispatchSemaphore(value: 0)
    let words = Box<String>()
    let failed = Box<Bool>()
    let task = recognizer.recognitionTask(with: request) { result, error in
        if let result, result.isFinal { words.value = result.bestTranscription.formattedString; done.signal() }
        else if error != nil { failed.value = true; done.signal() }
    }
    done.wait()
    task.cancel()
    return failed.value == true && words.value == nil ? (Code.failed, "") : (Code.ready, words.value ?? "")
}

// MARK: - What `main.swift` asks

/// Where the language stands: a `Code`.
func stateOf(_ tag: String) -> String {
    let locale = requested(tag)
    if #available(macOS 26.0, *) { return blocking { await stateModern(locale) } }
    return stateLegacy(locale)
}

/// The language to hear when none is named: Apple's recognizers cannot find the language of a
/// clip themselves, so it is the first of the Mac's languages that is ready to hear now,
/// else English. Never one that would have to be downloaded.
func unnamedLanguage() -> String {
    Locale.preferredLanguages.first { stateOf($0) == Code.ready } ?? "en-US"
}

/// Install the language's model (macOS 26 and later; blocks until it is on this Mac).
func install(_ tag: String) -> String {
    let locale = requested(tag)
    if #available(macOS 26.0, *) { return blocking { await installModern(locale) } }
    return Code.needsSettings
}

/// The words in `samples`, and a `Code`.
func hear(_ tag: String, _ samples: [Int16]) -> (code: String, text: String) {
    let locale = requested(tag)
    if #available(macOS 26.0, *) { return blocking { await hearModern(locale, samples) } }
    return hearLegacy(locale, samples)
}
