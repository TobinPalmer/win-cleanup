mod view;

use std::{
    cmp::Reverse,
    collections::HashSet,
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

use crate::{
    scan::{self, Item, Report, Risk, Target},
    windows,
};
use KeyCode::*;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    List,
    Preview,
    Confirm,
    Cleaning,
    Report,
}

#[derive(Clone, Copy)]
enum Sort {
    Size,
    Name,
    Newest,
}

struct Directory {
    path: PathBuf,
    items: Vec<Item>,
    cursor: usize,
    scroll: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Category(usize),
    Item(usize, usize),
}

struct App {
    targets: Vec<Target>,
    selected: HashSet<usize>,
    expanded: HashSet<usize>,
    included: HashSet<PathBuf>,
    rows: Vec<Row>,
    directories: Vec<Directory>,
    screen: Screen,
    list_cursor: usize,
    list_scroll: usize,
    preview_cursor: usize,
    preview_scroll: usize,
    report_scroll: usize,
    query: String,
    search: bool,
    sort: Sort,
    help: bool,
    warnings: bool,
    warning_scroll: usize,
    confirm_recycle: bool,
    confirm_scroll: usize,
    status: String,
    scanner: Receiver<ScanEvent>,
    scanning: Vec<(usize, &'static str)>,
    scanned: usize,
    cleaning: Option<Receiver<Report>>,
    report: Report,
    device: u64,
    page: usize,
}

enum ScanEvent {
    Started(usize, &'static str),
    Finished(usize, Target),
}

fn start_scan(targets: &[Target], device: u64, quick: bool) -> Receiver<ScanEvent> {
    let pending: Vec<_> = targets
        .iter()
        .map(|target| Target {
            id: target.id,
            name: target.name,
            risk: target.risk,
            roots: target.roots.clone(),
            items: Vec::new(),
            warnings: Vec::new(),
        })
        .collect();
    let (sender, receiver) = mpsc::channel();
    let queue = Arc::new(Mutex::new(pending.into_iter().enumerate()));
    for _ in 0..targets.len().min(4) {
        let queue = Arc::clone(&queue);
        let sender = sender.clone();
        thread::spawn(move || {
            loop {
                let next = queue.lock().unwrap().next();
                let Some((index, target)) = next else { break };
                if sender.send(ScanEvent::Started(index, target.name)).is_err() {
                    break;
                }
                let target = scan::scan_target(target, device, quick);
                if sender.send(ScanEvent::Finished(index, target)).is_err() {
                    break;
                }
            }
        });
    }
    receiver
}

impl App {
    fn new(targets: Vec<Target>, device: u64) -> Self {
        Self {
            scanner: start_scan(&targets, device, true),
            scanning: Vec::new(),
            targets,
            device,
            selected: HashSet::new(),
            expanded: HashSet::new(),
            included: HashSet::new(),
            rows: Vec::new(),
            directories: Vec::new(),
            screen: Screen::List,
            list_cursor: 0,
            list_scroll: 0,
            preview_cursor: 0,
            preview_scroll: 0,
            report_scroll: 0,
            query: String::new(),
            search: false,
            sort: Sort::Size,
            help: false,
            warnings: false,
            warning_scroll: 0,
            confirm_recycle: false,
            confirm_scroll: 0,
            status: String::new(),
            scanned: 0,
            cleaning: None,
            report: Report::default(),
            page: 10,
        }
    }

    fn scan_pending(&self) -> bool {
        self.scanned < self.targets.len()
    }

    fn update(&mut self) {
        for event in self.scanner.try_iter() {
            match event {
                ScanEvent::Started(index, name) => self.scanning.push((index, name)),
                ScanEvent::Finished(index, target) => {
                    self.scanning.retain(|(active, _)| *active != index);
                    self.targets[index] = target;
                    self.scanned += 1;
                    if !self.scan_pending()
                        && self.screen == Screen::List
                        && self.selected.is_empty()
                    {
                        self.targets.sort_by_key(|target| Reverse(target.bytes()));
                    }
                }
            }
        }
        if let Some(receiver) = &self.cleaning {
            match receiver.try_recv() {
                Ok(report) => self.report = report,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.report.errors.push(
                        "Cleanup worker stopped. Check the Recycle Bin before retrying.".into(),
                    );
                }
            }
            self.cleaning = None;
            self.screen = Screen::Report;
        }
    }

    fn plan(&self) -> impl Iterator<Item = (&'static str, &Item)> {
        self.targets
            .iter()
            .enumerate()
            .filter(|(index, _)| self.selected.contains(index))
            .flat_map(|(_, target)| target.items.iter().map(|item| (target.id, item)))
            .filter(|(_, item)| self.included.contains(&item.path))
    }

    fn begin_preview(&mut self) {
        if self.selected.is_empty() {
            self.status = "Select a category with Space first.".into();
            return;
        }
        self.included.clear();
        for index in &self.selected {
            let target = &self.targets[*index];
            if target.risk != Risk::Risky {
                self.included
                    .extend(target.items.iter().map(|item| item.path.clone()));
            }
        }
        self.expanded.clear();
        self.query.clear();
        self.preview_cursor = 0;
        self.preview_scroll = 0;
        self.screen = Screen::Preview;
        self.rebuild_rows(None);
    }

    fn rebuild_rows(&mut self, focus: Option<Row>) {
        let query = self.query.to_lowercase();
        self.rows.clear();
        for (index, target) in self.targets.iter().enumerate() {
            if !self.selected.contains(&index) {
                continue;
            }
            self.rows.push(Row::Category(index));
            if self.expanded.contains(&index) || !query.is_empty() {
                let mut items: Vec<_> = target
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| {
                        query.is_empty()
                            || item.path.to_string_lossy().to_lowercase().contains(&query)
                    })
                    .collect();
                items.sort_by(|(_, a), (_, b)| compare(self.sort, a, b));
                self.rows
                    .extend(items.into_iter().map(|(item, _)| Row::Item(index, item)));
            }
        }
        self.preview_cursor = focus
            .and_then(|row| self.rows.iter().position(|entry| *entry == row))
            .unwrap_or(self.preview_cursor)
            .min(self.rows.len().saturating_sub(1));
    }

    fn active_path(&self) -> Option<&Item> {
        if let Some(dir) = self.directories.last() {
            return dir.items.get(dir.cursor);
        }
        match self.rows.get(self.preview_cursor)? {
            Row::Item(category, item) => self.targets[*category].items.get(*item),
            Row::Category(_) => None,
        }
    }

    fn drill(&mut self, path: PathBuf) {
        match scan::children(&path, self.device) {
            Ok(mut items) => {
                items.sort_by(|a, b| compare(self.sort, a, b));
                self.directories.push(Directory {
                    path,
                    items,
                    cursor: 0,
                    scroll: 0,
                });
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    fn rescan(&mut self, quick: bool) {
        self.scanner = start_scan(&self.targets, self.device, quick);
        self.scanning.clear();
        self.scanned = 0;
        self.selected.clear();
        self.included.clear();
        self.list_cursor = 0;
        self.list_scroll = 0;
        self.report_scroll = 0;
        self.report = Report::default();
        self.status.clear();
        for target in &mut self.targets {
            target.items.clear();
            target.warnings.clear();
        }
        self.screen = Screen::List;
    }

    fn handle_list(&mut self, key: KeyCode) -> bool {
        let index = self.list_cursor;
        match key {
            Char('q') | Esc => return true,
            Up | Char('k') => move_cursor(&mut self.list_cursor, -1, self.targets.len()),
            Down | Char('j') => move_cursor(&mut self.list_cursor, 1, self.targets.len()),
            Enter | Char('p') => self.begin_preview(),
            Char('f') if !self.scan_pending() => self.rescan(false),
            Char(' ') if !self.targets[index].items.is_empty() => {
                if !self.selected.remove(&index) {
                    self.selected.insert(index);
                }
            }
            Char('a') => {
                self.selected.extend(
                    self.targets
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| t.risk != Risk::Risky && !t.items.is_empty())
                        .map(|(i, _)| i),
                );
            }
            Char('d') => self.selected.clear(),
            _ => {}
        }
        false
    }

    fn handle_preview(&mut self, key: KeyCode, modifiers: KeyModifiers) -> bool {
        if self.search {
            match key {
                Enter => self.search = false,
                Esc => {
                    self.search = false;
                    self.query.clear();
                }
                Backspace => {
                    self.query.pop();
                }
                Char(c) => self.query.push(c),
                _ => {}
            }
            self.preview_cursor = 0;
            self.rebuild_rows(None);
            return false;
        }
        let step = match key {
            Up if modifiers.contains(KeyModifiers::SHIFT) => -(self.page as isize),
            Down if modifiers.contains(KeyModifiers::SHIFT) => self.page as isize,
            Up | Char('k') => -1,
            Down | Char('j') => 1,
            PageUp | Char('K') => -(self.page as isize),
            PageDown | Char('J') => self.page as isize,
            _ => 0,
        };
        if step != 0 {
            if let Some(dir) = self.directories.last_mut() {
                move_cursor(&mut dir.cursor, step, dir.items.len());
            } else {
                move_cursor(&mut self.preview_cursor, step, self.rows.len());
            }
            return false;
        }
        let row = self.rows[self.preview_cursor];
        let category = match row {
            Row::Category(i) | Row::Item(i, _) => i,
        };
        match key {
            Char('q') => return true,
            Esc => {
                if self.directories.pop().is_none() {
                    if self.query.is_empty() {
                        self.screen = Screen::List;
                    } else {
                        self.query.clear();
                        self.rebuild_rows(Some(row));
                    }
                }
            }
            Enter => {
                if let Some(item) = self.active_path() {
                    if item.directory {
                        self.drill(item.path.clone());
                    }
                } else if self.directories.is_empty() && matches!(row, Row::Category(_)) {
                    if !self.expanded.remove(&category) {
                        self.expanded.insert(category);
                    }
                    self.rebuild_rows(Some(row));
                }
            }
            Char('o') => {
                let path = self
                    .active_path()
                    .map(|item| {
                        if item.directory {
                            item.path.as_path()
                        } else {
                            item.path.parent().unwrap()
                        }
                    })
                    .or_else(|| self.directories.last().map(|dir| dir.path.as_path()))
                    .or_else(|| self.targets[category].roots.first().map(PathBuf::as_path));
                if let Some(path) = path
                    && let Err(err) = windows::open(path)
                {
                    self.status = err.to_string();
                }
            }
            Char('s') => {
                self.sort = match self.sort {
                    Sort::Size => Sort::Name,
                    Sort::Name => Sort::Newest,
                    Sort::Newest => Sort::Size,
                };
                if let Some(dir) = self.directories.last_mut() {
                    dir.items.sort_by(|a, b| compare(self.sort, a, b));
                    dir.cursor = 0;
                } else {
                    self.rebuild_rows(Some(row));
                }
            }
            _ if !self.directories.is_empty() => {}
            Left | Char('h') | Right | Char('l') => {
                if matches!(key, Left | Char('h')) {
                    self.expanded.remove(&category);
                } else {
                    self.expanded.insert(category);
                }
                self.rebuild_rows(Some(Row::Category(category)));
            }
            Tab | Char(']') | BackTab | Char('[') => {
                let next = if matches!(key, BackTab | Char('[')) {
                    self.rows
                        .iter()
                        .rposition(|r| matches!(r, Row::Category(i) if *i < category))
                } else {
                    self.rows
                        .iter()
                        .position(|r| matches!(r, Row::Category(i) if *i > category))
                };
                if let Some(index) = next {
                    self.preview_cursor = index;
                }
            }
            Char('/') => self.search = true,
            Char(' ') => {
                if let Row::Item(category, item) = row {
                    let path = &self.targets[category].items[item].path;
                    if !self.included.remove(path) {
                        self.included.insert(path.clone());
                    }
                } else if self.targets[category].risk != Risk::Risky {
                    let items = &self.targets[category].items;
                    let all = items.iter().all(|item| self.included.contains(&item.path));
                    for item in items {
                        if all {
                            self.included.remove(&item.path);
                        } else {
                            self.included.insert(item.path.clone());
                        }
                    }
                } else {
                    self.status = "Expand Old Downloads and include files individually.".into();
                }
            }
            Char('a') => {
                let query = self.query.to_lowercase();
                for index in &self.selected {
                    let target = &self.targets[*index];
                    if target.risk != Risk::Risky {
                        self.included.extend(
                            target
                                .items
                                .iter()
                                .filter(|item| {
                                    item.path.to_string_lossy().to_lowercase().contains(&query)
                                })
                                .map(|item| item.path.clone()),
                        );
                    }
                }
            }
            Char('d') => {
                let query = self.query.to_lowercase();
                self.included
                    .retain(|path| !path.to_string_lossy().to_lowercase().contains(&query));
            }
            Char('y') => {
                if self.included.is_empty() {
                    self.status = "Include at least one item first.".into();
                } else {
                    self.confirm_recycle = false;
                    self.confirm_scroll = 0;
                    self.screen = Screen::Confirm;
                }
            }
            _ => {}
        }
        false
    }

    fn handle(&mut self, event: KeyEvent) -> bool {
        if event.kind == KeyEventKind::Release || self.screen == Screen::Cleaning {
            return false;
        }
        if event.code == Char('c') && event.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        if self.help {
            if matches!(event.code, Char('?') | Esc | Char('q') | Enter) {
                self.help = false;
            }
            return false;
        }
        if self.warnings {
            let count = self.targets.iter().map(|t| t.warnings.len()).sum();
            match event.code {
                Char('w' | 'q') | Esc => self.warnings = false,
                Up | Char('k') => move_cursor(&mut self.warning_scroll, -1, count),
                Down | Char('j') => move_cursor(&mut self.warning_scroll, 1, count),
                _ => {}
            }
            return false;
        }
        if self.screen == Screen::List
            && event.code == Char('w')
            && self.targets.iter().any(|t| !t.warnings.is_empty())
        {
            self.warning_scroll = 0;
            self.warnings = true;
            return false;
        }
        if event.code == Char('?') && !self.search {
            self.help = true;
            return false;
        }
        self.status.clear();
        match self.screen {
            Screen::List => return self.handle_list(event.code),
            Screen::Preview => return self.handle_preview(event.code, event.modifiers),
            Screen::Confirm => match event.code {
                Char('q') => return true,
                Up | Char('k') => move_cursor(&mut self.confirm_scroll, -1, self.selected.len()),
                Down | Char('j') => move_cursor(&mut self.confirm_scroll, 1, self.selected.len()),
                Esc | Char('n') => self.screen = Screen::Preview,
                Left | Char('h') => self.confirm_recycle = false,
                Right | Char('l') => self.confirm_recycle = true,
                Tab | BackTab => self.confirm_recycle = !self.confirm_recycle,
                Char('y') | Enter if event.kind == KeyEventKind::Press => {
                    if event.code == Char('y') || self.confirm_recycle {
                        let plan = self.plan().map(|(id, item)| (id, item.clone())).collect();
                        let device = self.device;
                        let (sender, receiver) = mpsc::channel();
                        self.cleaning = Some(receiver);
                        self.screen = Screen::Cleaning;
                        thread::spawn(move || {
                            let _ = sender.send(scan::clean(plan, device));
                        });
                    } else {
                        self.screen = Screen::Preview;
                    }
                }
                _ => {}
            },
            Screen::Report => match event.code {
                Char('q') | Esc => return true,
                Up | Char('k') => {
                    move_cursor(&mut self.report_scroll, -1, self.report.errors.len())
                }
                Down | Char('j') => {
                    move_cursor(&mut self.report_scroll, 1, self.report.errors.len())
                }
                Enter => {
                    self.rescan(true);
                }
                _ => {}
            },
            Screen::Cleaning => {}
        }
        false
    }
}

fn compare(sort: Sort, a: &Item, b: &Item) -> std::cmp::Ordering {
    match sort {
        Sort::Size => b.bytes.cmp(&a.bytes),
        Sort::Name => a.path.file_name().cmp(&b.path.file_name()),
        Sort::Newest => b.modified.cmp(&a.modified),
    }
    .then_with(|| a.path.cmp(&b.path))
}

fn move_cursor(cursor: &mut usize, delta: isize, len: usize) {
    *cursor = cursor
        .saturating_add_signed(delta)
        .min(len.saturating_sub(1));
}

pub fn run(targets: Vec<Target>, device: u64) -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = (|| -> io::Result<()> {
        let mut app = App::new(targets, device);
        loop {
            app.update();
            terminal.draw(|frame| view::draw(frame, &mut app))?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
            {
                let area = terminal.size()?;
                if area.width >= 32 && area.height >= 10 {
                    if app.handle(key) {
                        return Ok(());
                    }
                } else if app.screen != Screen::Cleaning
                    && (key.code == Char('q')
                        || key.code == Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return Ok(());
                }
            }
        }
    })();
    let restored = ratatui::try_restore();
    result.and(restored)
}
