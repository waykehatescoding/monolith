#![windows_subsystem = "windows"]

mod install;
mod system;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::alignment::Horizontal;
use iced::widget::{
    button, checkbox, column, container, mouse_area, progress_bar, row, svg, text, text_input,
    Space,
};
use iced::{window, Alignment, Border, Element, Length, Size, Task};
use monolith_theme::token::*;
use monolith_theme::{icon, style, theme, Icon, FONT_BYTES, MONO, UI, UI_MEDIUM, UI_SEMIBOLD};

use install::{Payload, UNINSTALLER_NAME};

static PAYLOAD: Payload = Payload {
    exe: include_bytes!(concat!(env!("OUT_DIR"), "/payload_exe.bin")),
    plugin: include_bytes!(concat!(env!("OUT_DIR"), "/payload_plugin.bin")),
};

const MARK: &[u8] = include_bytes!("../../../assets/brand/mark.svg");

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Install,
    Update,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Prepare,
    Files,
    Uninstaller,
    Shortcuts,
    Register,
    Relaunch,
    RemoveFiles,
    RemoveShortcuts,
    Unregister,
    UserData,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Prepare => "Checking that Monolith is closed",
            Step::Files => "Copying program files",
            Step::Uninstaller => "Adding the uninstaller",
            Step::Shortcuts => "Creating shortcuts",
            Step::Register => "Registering with Windows",
            Step::Relaunch => "Starting Monolith",
            Step::RemoveFiles => "Removing program files",
            Step::RemoveShortcuts => "Removing shortcuts",
            Step::Unregister => "Unregistering from Windows",
            Step::UserData => "Deleting servers and settings",
        }
    }
}

fn plan(mode: Mode, delete_data: bool) -> Vec<Step> {
    match mode {
        Mode::Install => vec![
            Step::Prepare,
            Step::Files,
            Step::Uninstaller,
            Step::Shortcuts,
            Step::Register,
        ],
        Mode::Update => vec![
            Step::Prepare,
            Step::Files,
            Step::Uninstaller,
            Step::Register,
            Step::Relaunch,
        ],
        Mode::Remove => {
            let mut steps = vec![
                Step::Prepare,
                Step::RemoveFiles,
                Step::RemoveShortcuts,
                Step::Unregister,
            ];
            if delete_data {
                steps.push(Step::UserData);
            }
            steps
        }
    }
}

#[derive(Debug, Clone)]
enum Problem {
    Running,
    Failed(String),
}

#[derive(Debug, Clone)]
struct Job {
    root: PathBuf,
    desktop: bool,
    wait: bool,
}

fn failed(result: io::Result<()>) -> Result<(), Problem> {
    result.map_err(|error| Problem::Failed(error.to_string()))
}

fn run_step(step: Step, job: &Job) -> Result<(), Problem> {
    match step {
        Step::Prepare => {
            let closed = if job.wait {
                system::wait_for_app_exit(Duration::from_secs(30))
            } else {
                !system::app_running()
            };
            if closed {
                Ok(())
            } else {
                Err(Problem::Running)
            }
        }
        Step::Files => failed(install::install_files(&job.root, &PAYLOAD)),
        Step::Uninstaller => {
            let setup = std::env::current_exe().map_err(|e| Problem::Failed(e.to_string()))?;
            failed(install::write_uninstaller(&job.root, &setup))
        }
        Step::Shortcuts => failed(system::create_shortcuts(&job.root, job.desktop)),
        Step::Register => failed(system::register(&job.root)),
        Step::Relaunch => {
            system::launch(&job.root);
            Ok(())
        }
        Step::RemoveFiles => failed(install::remove_program_files(&job.root)),
        Step::RemoveShortcuts => {
            system::remove_shortcuts();
            Ok(())
        }
        Step::Unregister => {
            system::unregister();
            Ok(())
        }
        Step::UserData => failed(system::delete_user_data()),
    }
}

fn existing_root() -> PathBuf {
    let beside_uninstaller = std::env::current_exe().ok().and_then(|exe| {
        let named = exe.file_name().is_some_and(|name| name == UNINSTALLER_NAME);
        named.then(|| exe.parent().map(Path::to_path_buf)).flatten()
    });
    beside_uninstaller
        .or_else(system::installed_root)
        .unwrap_or_else(system::default_root)
}

fn detect_mode() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let named_uninstaller = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|name| name == UNINSTALLER_NAME))
        .unwrap_or(false);
    if has("--update") {
        Mode::Update
    } else if has("--uninstall") || named_uninstaller {
        Mode::Remove
    } else {
        Mode::Install
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Welcome,
    Options,
    Working,
    Done,
    Failed,
    Confirm,
    Removed,
}

struct Setup {
    mode: Mode,
    screen: Screen,
    root: String,
    desktop: bool,
    delete_data: bool,
    plan: Vec<Step>,
    current: usize,
    problem: Option<Problem>,
}

#[derive(Debug, Clone)]
enum Message {
    Drag,
    Minimize,
    Close,
    Continue,
    Back,
    Root(String),
    Browse,
    Picked(Option<PathBuf>),
    Desktop(bool),
    DeleteData(bool),
    Begin,
    Stepped(usize, Result<(), Problem>),
    CloseApp,
    Launch,
}

fn on_window(action: fn(window::Id) -> Task<Message>) -> Task<Message> {
    window::oldest().then(move |id| match id {
        Some(id) => action(id),
        None => Task::none(),
    })
}

impl Setup {
    fn new(mode: Mode) -> (Self, Task<Message>) {
        let root = match mode {
            Mode::Install => system::default_root(),
            _ => existing_root(),
        };
        let mut setup = Setup {
            mode,
            screen: match mode {
                Mode::Install => Screen::Welcome,
                Mode::Update => Screen::Working,
                Mode::Remove => Screen::Confirm,
            },
            root: root.display().to_string(),
            desktop: true,
            delete_data: false,
            plan: plan(mode, false),
            current: 0,
            problem: None,
        };
        let task = if mode == Mode::Update {
            setup.run(0)
        } else {
            Task::none()
        };
        (setup, task)
    }

    fn job(&self) -> Job {
        Job {
            root: PathBuf::from(self.root.trim()),
            desktop: self.desktop,
            wait: self.mode == Mode::Update,
        }
    }

    fn valid_root(&self) -> bool {
        let path = Path::new(self.root.trim());
        path.is_absolute() && path.parent().is_some() && !path.starts_with(r"C:\Windows")
    }

    fn run(&mut self, index: usize) -> Task<Message> {
        self.current = index;
        let step = self.plan[index];
        let job = self.job();
        let pause = self.mode != Mode::Update;
        Task::perform(
            async move {
                let result = tokio::task::spawn_blocking(move || run_step(step, &job))
                    .await
                    .unwrap_or_else(|error| Err(Problem::Failed(error.to_string())));
                if pause {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                result
            },
            move |result| Message::Stepped(index, result),
        )
    }

    fn begin(&mut self) -> Task<Message> {
        self.plan = plan(self.mode, self.delete_data);
        self.problem = None;
        self.screen = Screen::Working;
        self.run(0)
    }

    fn quit(&self) -> Task<Message> {
        if self.mode == Mode::Remove && self.screen == Screen::Removed {
            system::schedule_self_delete(&self.job().root);
        }
        iced::exit()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Drag => return on_window(window::drag),
            Message::Minimize => return on_window(|id| window::minimize(id, true)),
            Message::Close if self.screen == Screen::Working => {}
            Message::Close => return self.quit(),
            Message::Continue => self.screen = Screen::Options,
            Message::Back => self.screen = Screen::Welcome,
            Message::Root(value) => self.root = value,
            Message::Browse => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_folder()
                            .await
                            .map(|folder| folder.path().to_path_buf())
                    },
                    Message::Picked,
                )
            }
            Message::Picked(Some(folder)) => {
                let named = folder.file_name().is_some_and(|name| name == "Monolith");
                let target = if named {
                    folder
                } else {
                    folder.join("Monolith")
                };
                self.root = target.display().to_string();
            }
            Message::Picked(None) => {}
            Message::Desktop(value) => self.desktop = value,
            Message::DeleteData(value) => self.delete_data = value,
            Message::Begin => return self.begin(),
            Message::Stepped(index, Ok(())) => {
                if index + 1 < self.plan.len() {
                    return self.run(index + 1);
                }
                match self.mode {
                    Mode::Install => self.screen = Screen::Done,
                    Mode::Update => return iced::exit(),
                    Mode::Remove => self.screen = Screen::Removed,
                }
            }
            Message::Stepped(_, Err(problem)) => {
                self.problem = Some(problem);
                self.screen = Screen::Failed;
            }
            Message::CloseApp => {
                system::close_app();
                return self.begin();
            }
            Message::Launch => {
                system::launch(&self.job().root);
                return iced::exit();
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let body = match self.screen {
            Screen::Welcome => self.welcome(),
            Screen::Options => self.options(),
            Screen::Working => self.working(),
            Screen::Done => self.done(),
            Screen::Failed => self.failure(),
            Screen::Confirm => self.confirm(),
            Screen::Removed => self.removed(),
        };
        container(column![title_bar(), body])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme| container::Style {
                border: Border {
                    color: HAIRLINE,
                    width: BORDER,
                    radius: 0.0.into(),
                },
                ..style::ground(theme)
            })
            .into()
    }

    fn welcome(&self) -> Element<'_, Message> {
        let ready = !PAYLOAD.is_empty();
        let intro = if ready {
            text("Create, run and back up Minecraft servers from one window. Monolith installs for your account only and needs no administrator access.")
                .size(TEXT_LG)
                .style(style::muted)
        } else {
            text("Development build: no payload")
                .size(TEXT_LG)
                .style(style::colored(AMBER))
        };
        page(
            column![
                svg(svg::Handle::from_memory(MARK)).width(48).height(48),
                heading("Install Monolith"),
                intro.width(Length::Fill),
            ]
            .spacing(SPACE_3),
            actions(None, action("Continue", ready.then_some(Message::Continue))),
        )
    }

    fn options(&self) -> Element<'_, Message> {
        let valid = self.valid_root();
        let field = text_input(r"C:\Users\you\AppData\Local\Programs\Monolith", &self.root)
            .on_input(Message::Root)
            .font(MONO)
            .size(TEXT_MD)
            .padding([8, 12])
            .style(move |theme, status| {
                if valid {
                    style::input(theme, status)
                } else {
                    style::input_error(theme, status)
                }
            });
        let browse = button(text("Browse").size(TEXT_MD).font(UI_MEDIUM))
            .padding([8, 16])
            .style(style::secondary)
            .on_press(Message::Browse);
        let mut location = column![
            text("Install location").size(TEXT_SM).style(style::muted),
            row![field, browse].spacing(SPACE_2),
        ]
        .spacing(SPACE_1);
        if !valid {
            location = location.push(
                text("Enter a full folder path, such as C:\\Users\\you\\Monolith.")
                    .size(TEXT_SM)
                    .style(style::colored(CORAL_HOVER)),
            );
        }
        page(
            column![
                heading("Choose where Monolith goes"),
                location,
                checkbox(self.desktop)
                    .label("Add a desktop shortcut")
                    .size(18)
                    .text_size(TEXT_MD)
                    .style(style::check)
                    .on_toggle(Message::Desktop),
                text("A Start menu shortcut is always added.")
                    .size(TEXT_SM)
                    .style(style::muted),
            ]
            .spacing(SPACE_4),
            actions(
                Some(Message::Back),
                action("Install", valid.then_some(Message::Begin)),
            ),
        )
    }

    fn working(&self) -> Element<'_, Message> {
        let title = match self.mode {
            Mode::Install => "Installing Monolith",
            Mode::Update => "Updating Monolith",
            Mode::Remove => "Removing Monolith",
        };
        let value = self.current as f32 / self.plan.len() as f32;
        let label = self.plan.get(self.current).map_or("", |step| step.label());
        let content = column![
            heading(title),
            progress_bar(0.0..=1.0, value)
                .girth(6)
                .style(style::progress),
            text(label).size(TEXT_MD).style(style::muted),
            text(self.root.trim())
                .size(TEXT_SM)
                .font(MONO)
                .style(style::muted),
        ]
        .spacing(SPACE_3);
        page(content, Space::new().into())
    }

    fn done(&self) -> Element<'_, Message> {
        page(
            column![
                heading("Monolith is installed"),
                text("Find it in the Start menu, or open it now.")
                    .size(TEXT_LG)
                    .style(style::muted),
                text(self.root.trim())
                    .size(TEXT_SM)
                    .font(MONO)
                    .style(style::muted),
            ]
            .spacing(SPACE_3),
            actions(
                Some(Message::Close),
                action("Launch Monolith", Some(Message::Launch)),
            ),
        )
    }

    fn failure(&self) -> Element<'_, Message> {
        let (title, detail, primary) = match &self.problem {
            Some(Problem::Running) => (
                "Monolith is still running",
                "Setup cannot replace files while Monolith is open. Close it from here, or quit it yourself and try again.".to_string(),
                action("Close Monolith and retry", Some(Message::CloseApp)),
            ),
            Some(Problem::Failed(error)) => (
                "Setup could not finish",
                format!("{error}. Check that the folder can be written to and has free space, or choose another folder."),
                action("Try again", Some(Message::Begin)),
            ),
            None => (
                "Setup could not finish",
                "Something went wrong. Try again.".to_string(),
                action("Try again", Some(Message::Begin)),
            ),
        };
        page(
            column![
                heading(title),
                text(detail)
                    .size(TEXT_LG)
                    .style(style::muted)
                    .width(Length::Fill),
            ]
            .spacing(SPACE_3),
            actions(Some(Message::Close), primary),
        )
    }

    fn confirm(&self) -> Element<'_, Message> {
        let data = system::user_data_dir()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default();
        page(
            column![
                heading("Remove Monolith?"),
                text("This removes the app and its shortcuts. Your servers and settings stay on disk unless you choose to delete them.")
                    .size(TEXT_LG)
                    .style(style::muted)
                    .width(Length::Fill),
                checkbox(self.delete_data)
                    .label("Also delete my servers and settings")
                    .size(18)
                    .text_size(TEXT_MD)
                    .style(style::check)
                    .on_toggle(Message::DeleteData),
                text(data).size(TEXT_SM).font(MONO).style(style::muted),
            ]
            .spacing(SPACE_3),
            actions(
                Some(Message::Close),
                button(
                    text("Remove Monolith")
                        .size(TEXT_LG)
                        .font(UI_SEMIBOLD)
                        .width(Length::Fill)
                        .align_x(Horizontal::Center),
                )
                .width(240)
                .padding([13, 16])
                .style(style::danger)
                .on_press(Message::Begin)
                .into(),
            ),
        )
    }

    fn removed(&self) -> Element<'_, Message> {
        let detail = if self.delete_data {
            "Program files, shortcuts and your servers were deleted."
        } else {
            "Your servers and settings were kept, so a reinstall picks up where you left off."
        };
        page(
            column![
                heading("Monolith was removed"),
                text(detail)
                    .size(TEXT_LG)
                    .style(style::muted)
                    .width(Length::Fill),
            ]
            .spacing(SPACE_3),
            actions(None, action("Close", Some(Message::Close))),
        )
    }
}

fn heading<'a>(value: &'a str) -> Element<'a, Message> {
    text(value).size(TEXT_2XL).font(UI_SEMIBOLD).into()
}

fn page<'a>(
    content: impl Into<Element<'a, Message>>,
    footer: Element<'a, Message>,
) -> Element<'a, Message> {
    column![content.into(), Space::new().height(Length::Fill), footer]
        .padding(SPACE_6)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn action<'a>(label: &'a str, message: Option<Message>) -> Element<'a, Message> {
    button(
        text(label)
            .size(TEXT_LG)
            .font(UI_SEMIBOLD)
            .width(Length::Fill)
            .align_x(Horizontal::Center),
    )
    .width(240)
    .padding([13, 16])
    .style(style::primary)
    .on_press_maybe(message)
    .into()
}

fn actions<'a>(secondary: Option<Message>, primary: Element<'a, Message>) -> Element<'a, Message> {
    let mut bar = row![].spacing(SPACE_3).align_y(Alignment::Center);
    if let Some(message) = secondary {
        let label = if matches!(message, Message::Back) {
            "Back"
        } else {
            "Close"
        };
        bar = bar.push(
            button(text(label).size(TEXT_MD).font(UI_MEDIUM))
                .padding([13, 16])
                .style(style::ghost)
                .on_press(message),
        );
    }
    bar.push(Space::new().width(Length::Fill))
        .push(primary)
        .into()
}

fn title_bar<'a>() -> Element<'a, Message> {
    let control = |glyph: Icon, message: Message| {
        button(icon(glyph, 16.0))
            .padding([12, 14])
            .style(style::icon_button)
            .on_press(message)
    };
    container(row![
        mouse_area(
            container(text("Monolith Setup").size(TEXT_SM).style(style::muted))
                .padding([0, 16])
                .center_y(Length::Fill)
                .width(Length::Fill)
        )
        .on_press(Message::Drag),
        control(Icon::Minimize, Message::Minimize),
        control(Icon::Close, Message::Close),
    ])
    .height(40)
    .width(Length::Fill)
    .style(style::surface)
    .into()
}

fn quiet_remove() -> bool {
    let job = Job {
        root: existing_root(),
        desktop: false,
        wait: false,
    };
    system::close_app();
    system::wait_for_app_exit(Duration::from_secs(5));
    let mut clean = true;
    for step in plan(Mode::Remove, false) {
        if step != Step::Prepare {
            clean &= run_step(step, &job).is_ok();
        }
    }
    if clean {
        system::schedule_self_delete(&job.root);
    }
    clean
}

fn main() -> iced::Result {
    let mode = detect_mode();
    if mode == Mode::Remove && std::env::args().any(|arg| arg == "--quiet") {
        if !quiet_remove() {
            std::process::exit(1);
        }
        return Ok(());
    }
    let size = match mode {
        Mode::Update => Size::new(480.0, 300.0),
        _ => Size::new(640.0, 420.0),
    };
    let mut app = iced::application(move || Setup::new(mode), Setup::update, Setup::view)
        .title("Monolith Setup")
        .theme(|_: &Setup| theme())
        .default_font(UI)
        .window(window::Settings {
            size,
            resizable: false,
            decorations: false,
            position: window::Position::Centered,
            ..window::Settings::default()
        });
    for bytes in FONT_BYTES {
        app = app.font(bytes);
    }
    app.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_plan_deletes_data_only_when_asked() {
        assert!(!plan(Mode::Remove, false).contains(&Step::UserData));
        assert!(plan(Mode::Remove, true).contains(&Step::UserData));
    }

    #[test]
    fn update_plan_relaunches_without_shortcuts() {
        let steps = plan(Mode::Update, false);
        assert_eq!(steps.last(), Some(&Step::Relaunch));
        assert!(!steps.contains(&Step::Shortcuts));
    }
}
