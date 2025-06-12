/// The central trait that all plugins must implement.
/// It defines the lifecycle hooks for a plugin.
pub trait Plugin {
    /// A unique name for the plugin.
    fn name(&self) -> &'static str;

    /// Called at startup to initialize the plugin.
    /// The plugin can get shared facilities from the AppContext and
    /// register its own capabilities or providers.
    fn initialize(&mut self /*, context: AppContext */);

    /// Called after all plugins are initialized.
    /// If the plugin exposes a network service, it should be started here.
    fn start(&self);
}

// The AppContext and other core types will be defined here. 