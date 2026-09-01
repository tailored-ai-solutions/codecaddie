// Xcode needs a native executable product so its archive and cloud-managed
// Developer ID signing pipeline owns the final application seal. The build
// phase replaces this placeholder with CodeCaddie's universal Zig executable.
int main(void) {
    return 0;
}
