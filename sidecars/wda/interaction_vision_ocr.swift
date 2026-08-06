import AppKit
import Foundation
import Vision

guard CommandLine.arguments.count == 2 else { exit(2) }
guard let image = NSImage(contentsOfFile: CommandLine.arguments[1]) else { exit(3) }
var proposedRect = NSRect(origin: .zero, size: image.size)
guard let cgImage = image.cgImage(forProposedRect: &proposedRect, context: nil, hints: nil) else { exit(4) }

let request = VNRecognizeTextRequest()
request.revision = 3
request.recognitionLevel = .accurate
request.usesLanguageCorrection = true
request.recognitionLanguages = ["en-US", "vi-VN"]
try VNImageRequestHandler(cgImage: cgImage, options: [:]).perform([request])

var output: [[String: Any]] = []
for observation in request.results ?? [] {
    guard let candidate = observation.topCandidates(1).first else { continue }
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
FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: output, options: [.sortedKeys]))
