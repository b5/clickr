import Foundation

/// The nickname this phone introduces itself with, persisted in `UserDefaults`.
///
/// iOS only reports a generic "iPhone" as the device name, so the first launch picks a
/// random two-word name instead; the user can rename it in settings.
enum PhoneProfile {
    private static let key = "phoneNickname"

    private static let adjectives = [
        "Amber", "Bold", "Brave", "Bright", "Calm", "Clever", "Cosmic", "Crisp", "Swift",
        "Gentle", "Golden", "Happy", "Jolly", "Lucky", "Mellow", "Misty", "Nimble", "Quiet",
        "Rapid", "Rusty", "Silver", "Sunny", "Velvet", "Witty",
    ]
    private static let animals = [
        "Badger", "Beaver", "Falcon", "Ferret", "Fox", "Gecko", "Heron", "Ibis", "Koala",
        "Lemur", "Lynx", "Marten", "Narwhal", "Otter", "Owl", "Panda", "Puffin", "Quokka",
        "Raven", "Seal", "Sparrow", "Tapir", "Walrus", "Wombat",
    ]

    /// Returns the saved nickname, generating and saving one on first use.
    static func nickname() -> String {
        if let saved = UserDefaults.standard.string(forKey: key), !saved.isEmpty {
            return saved
        }
        let generated = "\(adjectives.randomElement()!) \(animals.randomElement()!)"
        setNickname(generated)
        return generated
    }

    static func setNickname(_ nickname: String) {
        UserDefaults.standard.set(nickname, forKey: key)
    }
}
