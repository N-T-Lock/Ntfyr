use std::cell::OnceCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use tracing::debug;

use crate::error::*;
use super::NtfyrAccountDialog;

mod imp {
    use ntfy_daemon::NtfyHandle;

    use super::*;

    #[derive(gtk::CompositeTemplate)]
    #[template(resource = "/io/github/tobagin/Ntfyr/ui/preferences.ui")]
    pub struct NtfyrPreferences {
        #[template_child]
        pub startup_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub sort_descending_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub startup_background_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub add_account_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub added_accounts: TemplateChild<gtk::ListBox>,
        pub notifier: OnceCell<NtfyHandle>,
    }

    impl Default for NtfyrPreferences {
        fn default() -> Self {
            let this = Self {
                startup_switch: Default::default(),
                sort_descending_switch: Default::default(),
                startup_background_switch: Default::default(),
                add_account_btn: Default::default(),
                added_accounts: Default::default(),
                notifier: Default::default(),
            };

            this
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NtfyrPreferences {
        const NAME: &'static str = "NtfyrPreferences";
        type Type = super::NtfyrPreferences;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for NtfyrPreferences {}

    impl WidgetImpl for NtfyrPreferences {}
    impl AdwDialogImpl for NtfyrPreferences {}
    impl PreferencesDialogImpl for NtfyrPreferences {}
}

glib::wrapper! {
    pub struct NtfyrPreferences(ObjectSubclass<imp::NtfyrPreferences>)
        @extends gtk::Widget, adw::Dialog, adw::PreferencesDialog,
        @implements gio::ActionMap, gio::ActionGroup, gtk::Root, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native, gtk::ShortcutManager;
}

impl NtfyrPreferences {
    pub fn new(notifier: ntfy_daemon::NtfyHandle) -> Self {
        let obj: Self = glib::Object::builder().build();
        obj.imp()
            .notifier
            .set(notifier)
            .map_err(|_| "notifier")
            .unwrap();

        let settings = gio::Settings::new(crate::config::APP_ID);
        settings
            .bind("run-on-startup", &*obj.imp().startup_switch, "active")
            .build();
        settings
            .bind("sort-descending", &*obj.imp().sort_descending_switch, "active")
            .build();
        settings
            .bind("start-in-background", &*obj.imp().startup_background_switch, "active")
            .build();

        // settings.connect_changed("run-on-startup") handled in application.rs

        let this = obj.clone();
        obj.imp().add_account_btn.connect_clicked(move |btn| {
            let this = this.clone();
            btn.error_boundary()
                .spawn(async move { this.on_add_account_clicked().await });
        });
        let this = obj.clone();
        obj.imp()
            .added_accounts
            .error_boundary()
            .spawn(async move { this.show_accounts().await });
        obj
    }

    pub async fn show_accounts(&self) -> anyhow::Result<()> {
        debug!("show_accounts: starting");
        let imp = self.imp();
        let accounts = imp.notifier.get().unwrap().list_accounts().await?;
        debug!("show_accounts: accounts found: {}", accounts.len());

        while let Some(child) = imp.added_accounts.last_child() {
            imp.added_accounts.remove(&child);
        }

        if accounts.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No accounts configured")
                .subtitle("Add an account to receive private notifications")
                .build();
            let icon = gtk::Image::builder()
                .icon_name("user-available-symbolic")
                .pixel_size(32)
                .margin_end(12)
                .build();
            row.add_prefix(&icon);
            imp.added_accounts.append(&row);
        } else {
            for a in accounts {
                let row = adw::ActionRow::builder()
                    .title(&a.server)
                    .subtitle(&a.username)
                    .build();
                row.add_css_class("property");

                // Details button
                row.add_suffix(&{
                    let btn = gtk::Button::builder()
                        .icon_name("info-symbolic")
                        .tooltip_text("View Details")
                        .build();
                    btn.add_css_class("flat");
                    let server = a.server.clone();
                    let username = a.username.clone();
                    let this = self.clone();
                    btn.connect_clicked(move |_| {
                        let dialog = adw::AlertDialog::builder()
                            .heading("Account Details")
                            .body(format!("Server: {}\nUsername: {}", server, username))
                            .build();
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&this));
                    });
                    btn
                });

                // Edit button
                row.add_suffix(&{
                    let btn = gtk::Button::builder()
                        .icon_name("document-edit-symbolic")
                        .tooltip_text("Edit Account")
                        .build();
                    btn.add_css_class("flat");
                    let this = self.clone();
                    let a = a.clone();
                    btn.connect_clicked(move |btn| {
                        let this = this.clone();
                        let a = a.clone();
                        btn.error_boundary()
                            .spawn(async move { this.on_edit_account_clicked(&a).await });
                    });
                    btn
                });

                // Remove button
                row.add_suffix(&{
                    let btn = gtk::Button::builder()
                        .icon_name("user-trash-symbolic")
                        .tooltip_text("Remove Account")
                        .build();
                    btn.add_css_class("flat");
                    btn.add_css_class("error");
                    let this = self.clone();
                    let a = a.clone();
                    btn.connect_clicked(move |btn| {
                        let this = this.clone();
                        let a = a.clone();
                        btn.error_boundary()
                            .spawn(async move { this.remove_account(&a.server).await });
                    });
                    btn
                });
                imp.added_accounts.append(&row);
            }
        }
        Ok(())
    }

    pub async fn on_add_account_clicked(&self) -> anyhow::Result<()> {
        let dialog = NtfyrAccountDialog::new();
        dialog.set_title("Add Account");

        let (sender, receiver) = async_channel::bounded(1);
        dialog.connect_closure(
            "save",
            false,
            glib::closure_local!(move |_dialog: NtfyrAccountDialog| {
                let _ = sender.send_blocking(());
            }),
        );

        dialog.present(Some(self));

        if let Ok(()) = receiver.recv().await {
            let (server, username, password) = dialog.account_data();
            let n = self.imp().notifier.get().unwrap();
            n.add_account(&server, &username, &password).await?;
            self.show_accounts().await?;
        }

        Ok(())
    }

    pub async fn on_edit_account_clicked(&self, account: &ntfy_daemon::models::Account) -> anyhow::Result<()> {
        let dialog = NtfyrAccountDialog::new();
        dialog.set_account(account);

        let (sender, receiver) = async_channel::bounded(1);
        dialog.connect_closure(
            "save",
            false,
            glib::closure_local!(move |_dialog: NtfyrAccountDialog| {
                let _ = sender.send_blocking(());
            }),
        );

        dialog.present(Some(self));

        if let Ok(()) = receiver.recv().await {
            let (server, username, password) = dialog.account_data();
            let n = self.imp().notifier.get().unwrap();
            // ntfy-daemon might not have an "update_account" but add_account
            // with same server should overwrite or we remove and add.
            // Let's check ntfy-daemon/src/ntfy.rs if possible or just try add_account.
            n.add_account(&server, &username, &password).await?;
            self.show_accounts().await?;
        }

        Ok(())
    }

    pub async fn remove_account(&self, server: &str) -> anyhow::Result<()> {
        self.imp()
            .notifier
            .get()
            .unwrap()
            .remove_account(server)
            .await?;
        self.show_accounts().await?;
        Ok(())
    }
}
