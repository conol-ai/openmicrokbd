import CryptoKit
import Foundation

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: sparkle-public-key.swift PRIVATE_KEY_FILE\n".utf8))
    exit(2)
}

let keyURL = URL(fileURLWithPath: CommandLine.arguments[1])
let encoded = try String(contentsOf: keyURL, encoding: .utf8)
    .trimmingCharacters(in: .whitespacesAndNewlines)
guard let seed = Data(base64Encoded: encoded), seed.count == 32 else {
    FileHandle.standardError.write(Data("invalid Sparkle Ed25519 private key\n".utf8))
    exit(1)
}

let privateKey = try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
print(privateKey.publicKey.rawRepresentation.base64EncodedString())
