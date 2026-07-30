import AppKit
import Foundation
import Vision

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: vision_ocr.swift FRAME.jpg\n".utf8))
    exit(2)
}

let imagePath = CommandLine.arguments[1]
guard let image = NSImage(contentsOfFile: imagePath) else {
    FileHandle.standardError.write(Data("invalid image\n".utf8))
    exit(3)
}

var proposedRect = NSRect(origin: .zero, size: image.size)
guard let cgImage = image.cgImage(forProposedRect: &proposedRect, context: nil, hints: nil) else {
    FileHandle.standardError.write(Data("missing CGImage\n".utf8))
    exit(4)
}

let request = VNRecognizeTextRequest()
request.revision = 3
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true
request.recognitionLanguages = ["en-US", "vi-VN"]

do {
    try VNImageRequestHandler(cgImage: cgImage, options: [:]).perform([request])
} catch {
    FileHandle.standardError.write(Data("Vision request failed\n".utf8))
    exit(5)
}

var output: [[String: Any]] = []
for observation in request.results ?? [] {
    guard let candidate = observation.topCandidates(1).first else {
        continue
    }
    let box = observation.boundingBox
    output.append([
        "text": candidate.string,
        "confidence": Double(candidate.confidence),
        "x": Double(box.origin.x),
        "y": Double(1.0 - box.origin.y - box.height),
        "width": Double(box.width),
        "height": Double(box.height),
    ])
}

guard JSONSerialization.isValidJSONObject(output),
      let encoded = try? JSONSerialization.data(withJSONObject: output, options: [.sortedKeys]) else {
    FileHandle.standardError.write(Data("JSON encoding failed\n".utf8))
    exit(6)
}
FileHandle.standardOutput.write(encoded)
