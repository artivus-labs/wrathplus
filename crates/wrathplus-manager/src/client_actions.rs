//! Native folder chooser. Cancel preserves the existing selection and validation state.
use super::{Manager, Message};

impl Manager {
    pub(super) fn client_control(&self) -> iced::Element<'_, Message> {
        use iced::widget::button;
        let running = self.child.is_some();
        button(if running {
            "Stop Client"
        } else {
            "Start Client"
        })
        .style(super::style::primary)
        .padding([8, 16])
        .on_press_maybe(
            (!self.busy && (running || super::setup::available(&self.config))).then_some(
                if running {
                    Message::StopClient
                } else {
                    Message::Launch
                },
            ),
        )
        .into()
    }

    pub(super) fn choose_build_directory(&mut self) -> iced::Task<Message> {
        self.choosing = true;
        let directory = self.config.target.clone();
        iced::Task::perform(
            async move {
                let mut dialog =
                    rfd::AsyncFileDialog::new().set_title("Select build / client directory");
                if directory.is_dir() {
                    dialog = dialog.set_directory(directory);
                }
                dialog
                    .pick_folder()
                    .await
                    .map(|handle| handle.path().to_owned())
            },
            Message::BuildDirectoryChosen,
        )
    }

    pub(super) fn choose_assets(&mut self) -> iced::Task<Message> {
        self.choosing = true;
        let directory = std::path::PathBuf::from(&self.assets);
        iced::Task::perform(
            async move {
                let mut dialog =
                    rfd::AsyncFileDialog::new().set_title("Select your Wrath 3.3.5a Data folder");
                if directory.is_dir() {
                    dialog = dialog.set_directory(directory);
                }
                dialog.pick_folder().await.map(|handle| {
                    let path = handle.path().to_owned();
                    // Accept the install root too, when it contains the usual Data directory.
                    if path.join("Data").is_dir() {
                        path.join("Data")
                    } else {
                        path
                    }
                })
            },
            Message::FolderChosen,
        )
    }
}
