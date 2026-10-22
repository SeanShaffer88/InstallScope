//! Focused desktop shell. Retired tools have no launch route.
use super::{theme, tool_window};
use setupscope_core::{settings::Settings, store::Store};
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {Dashboard, ProjectReadiness, BuildLog, Environment, Ports, Containers, Settings, Doctor, Developer, Bundle, Dependencies}
pub(super) const NAVIGATION: &[(Tab,&str,&str,&str)] = &[
 (Tab::ProjectReadiness,"","Project Readiness","Project evidence, PATH/version checks and separately reviewed developer dependencies."),
 (Tab::BuildLog,"","Build Troubleshooting","Reviewed build/test execution, captured logs, bounded triage and diagnostic export."),
 (Tab::Environment,"","Environment Profiles","Project environments, reviewed persistent changes and conflict-checked undo."),
 (Tab::Ports,"","Ports And Processes","Read-only listeners, process ownership and file holders."),
 (Tab::Containers,"","WSL And Containers","Passive Windows host, WSL and container readiness evidence."),
];
impl Tab {
 pub(super) fn features(self)->&'static [&'static str] {match self.owner(){
  Self::ProjectReadiness=>&["Project Requirements And Readiness","Toolchain, SDK, PATH And Version Checks","Dependencies, Offline Bundles And Reviewed Setup","Redacted Diagnostic Export"],
  Self::BuildLog=>&["Reviewed Build And Test Runs","Captured Logs And Failure Hints","Timeout, Cancellation And Diagnostic Export"],
  Self::Environment=>&["Project Environment Profiles","Reviewed Launch And User Environment Changes","Conflict-Checked Undo And Recovery"],
  Self::Ports=>&["Local Listeners And Process Owners","Selected File Holders","Read-Only Filtering And Reports"],
  Self::Containers=>&["Windows Virtualization Prerequisites","WSL And Container Tool Evidence","Passive Service And Host Checks"],
  _=>&[],
 }}
 fn owner(self)->Self {match self {Self::Doctor|Self::Developer|Self::Bundle|Self::Dependencies=>Self::ProjectReadiness,other=>other}}
 fn title(self)-> &'static str {NAVIGATION.iter().find(|(t,_,_,_)|*t==self.owner()).map(|(_,_,s,_)|*s).unwrap_or("Settings")}
}
pub struct App {
 pub store:Store, pub settings:Settings, pub devcheck:super::devcheck_ui::DevCheckState,
 pub readiness:super::project_readiness_ui::ReadinessState, pub tool_windows:Vec<Tab>,
 readiness_page:Tab, status:String, pub(super) system_info:super::system_info::SystemInfo, board_hidden:bool,
}
impl App {
 pub fn new(cc:&eframe::CreationContext<'_>,_:Option<String>,_:Vec<String>)->anyhow::Result<Self> {
  let store=Store::open()?;
  let settings=store.load_settings();
  setupscope_core::i18n::set_language(&settings.language);
  theme::apply(&cc.egui_ctx,settings.high_contrast);
  cc.egui_ctx.set_zoom_factor(settings.zoom.clamp(0.75,2.));
  Ok(Self {store,settings,devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false})
 }
 pub fn set_status(&mut self,text:impl Into<String>) {self.status=text.into();}
 pub fn open_tool(&mut self,ctx:&egui::Context,tab:Tab) {
  let owner=tab.owner();
  if owner==Tab::ProjectReadiness && (!self.tool_windows.contains(&owner) || tab!=Tab::ProjectReadiness) {self.readiness_page=tab;}
  if !self.tool_windows.contains(&owner) {self.tool_windows.push(owner);}
  else {
   let id=egui::ViewportId::from_hash_of(format!("tool-{owner:?}"));
   ctx.send_viewport_cmd_to(id,egui::ViewportCommand::Minimized(false));
   ctx.send_viewport_cmd_to(id,egui::ViewportCommand::Focus);
  }
  self.board_hidden=true;
  ctx.send_viewport_cmd_to(egui::ViewportId::ROOT,egui::ViewportCommand::Visible(false));
  ctx.request_repaint();
 }
 fn show_board(&mut self,ctx:&egui::Context) {
  self.board_hidden=false;
  ctx.send_viewport_cmd_to(egui::ViewportId::ROOT,egui::ViewportCommand::Visible(true));
  ctx.send_viewport_cmd_to(egui::ViewportId::ROOT,egui::ViewportCommand::Minimized(false));
  ctx.send_viewport_cmd_to(egui::ViewportId::ROOT,egui::ViewportCommand::Focus);
  ctx.request_repaint();
 }
 fn close_tools(&mut self,ctx:&egui::Context,closed:&[Tab]) {
  self.tool_windows.retain(|t|!closed.contains(t));
  if self.tool_windows.is_empty() && self.board_hidden {self.show_board(ctx);}
 }
 fn render_tool(&mut self,ctx:&egui::Context,tab:Tab) {
  super::a11y::track_input_mode(ctx);
  egui::TopBottomPanel::top(egui::Id::new(("tool_header",format!("{tab:?}")))).frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::symmetric(10,6)).stroke(egui::Stroke::new(1.0_f32,theme::ACCENT_DIM))).show(ctx,|ui| {
   if tool_window::header(ui,if tab==Tab::ProjectReadiness && self.readiness_page==Tab::Dependencies {Tab::Dependencies}else{tab},if tab==Tab::ProjectReadiness && self.readiness_page==Tab::Dependencies {"Developer Dependencies"}else{tab.title()}) {self.show_board(ctx);}
   if tab==Tab::ProjectReadiness {ui.horizontal_wrapped(|ui| {
    for (page,label) in [(Tab::ProjectReadiness,"Project"),(Tab::Doctor,"Requirements"),(Tab::Developer,"Toolchain / PATH"),(Tab::Bundle,"Export"),(Tab::Dependencies,"Dependencies")] {
     if ui.selectable_label(self.readiness_page==page,setupscope_core::i18n::t(label)).on_hover_text(setupscope_core::i18n::t("Switch views inside this readiness window. Current project evidence is reused; no scan starts.")).clicked() {
      let accepted=if matches!(page,Tab::Doctor|Tab::Developer|Tab::Bundle) {
       self.readiness.current_project_evidence().map(|report| self.devcheck.accept_project_evidence(report)).unwrap_or(true)
      } else {true};
      if accepted {self.readiness_page=page;} else {self.status="A linked diagnostic is running; finish or cancel it before replacing its evidence.".into();}
     }
    }
   });}
  });
  match tab {
   Tab::ProjectReadiness=>match self.readiness_page {Tab::Doctor|Tab::Bundle|Tab::Dependencies=>self.ui_developer_tool(ctx,self.readiness_page),Tab::Developer=>self.ui_devcheck(ctx),_=>self.ui_project_readiness(ctx)},
   Tab::Settings=>{egui::CentralPanel::default().frame(theme::content_frame()).show(ctx,|ui| {tool_window::scroll_body(ui,"settings_body",|ui| {
    let old_language=self.settings.language.clone();
    let languages=setupscope_core::i18n::languages();
    egui::ComboBox::from_label(setupscope_core::i18n::t("Language"))
     .selected_text(if self.settings.language.is_empty(){setupscope_core::i18n::t("System")}else{languages.iter().find(|l|l.code==self.settings.language).map(|l|l.name.clone()).unwrap_or_else(||self.settings.language.clone())})
     .show_ui(ui,|ui|{
      ui.selectable_value(&mut self.settings.language,String::new(),setupscope_core::i18n::t("System"));
      for language in languages{ui.selectable_value(&mut self.settings.language,language.code,language.name);}
     }).response.on_hover_text(setupscope_core::i18n::t("Choose the interface language. Missing translations use English. Diagnostic evidence and command identifiers remain unchanged."));
    let language_changed=old_language!=self.settings.language;
    if language_changed {setupscope_core::i18n::set_language(&self.settings.language);}
    if setupscope_core::i18n::current_code()!="en" {ui.small(setupscope_core::i18n::t("Diagnostic evidence, command identifiers and vendor output keep their original wording."));}
    let mut changed=language_changed;
    changed|=ui.checkbox(&mut self.settings.high_contrast,setupscope_core::i18n::t("High Contrast")).changed();
    changed|=ui.add(egui::Slider::new(&mut self.settings.zoom,0.75..=2.).text(setupscope_core::i18n::t("Interface Scale"))).changed();
    if changed {theme::apply(ctx,self.settings.high_contrast);ctx.set_zoom_factor(self.settings.zoom);if let Err(e)=self.store.save_settings(&self.settings){self.status=format!("Could not save appearance: {e}");}}
    ui.separator(); ui.label(setupscope_core::i18n::t("Diagnostics are local. Downloads require explicit action; installation requires a separate review. Nothing uploads."));
    ui.label(setupscope_core::i18n::t("Saved settings, profiles and recovery records remain in your data folder. Review exported files and receipts before sharing."));
    ui.label(setupscope_core::i18n::t(&format!("Data Folder: {}",self.store.root.display())));
   });});},
   _=>self.ui_developer_tool(ctx,tab),
  }
  tool_window::fit_content(ctx,tab);
 }
}
impl App {
 fn ui_main_header(&mut self,ctx:&egui::Context) {
  egui::TopBottomPanel::top("menu").show(ctx,|ui| {ui.horizontal(|ui| {
   ui.colored_label(theme::ACCENT_TEXT,egui::RichText::new(setupscope_core::i18n::t("SetupScope")).monospace().size(18.).strong());
   ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
    if ui.button(setupscope_core::i18n::t("Settings")).on_hover_text(setupscope_core::i18n::t("Open appearance settings. The board hides while settings is open; use Workflow Board to return.")).clicked(){self.open_tool(ctx,Tab::Settings);}
   });
  });ui.label(egui::RichText::new(setupscope_core::i18n::t("Windows Software Development Toolkit")).small().color(theme::MUTED));});
 }
}
impl eframe::App for App {
 fn update(&mut self,ctx:&egui::Context,_:&mut eframe::Frame) {
  super::a11y::track_input_mode(ctx);
  self.ui_main_header(ctx);
  let status_panel=egui::TopBottomPanel::bottom("system_status").show(ctx,|ui| {ui.small(self.system_info.localized_summary());if !self.status.is_empty(){ui.label(setupscope_core::i18n::t(&self.status));}});
  ctx.data_mut(|d|d.insert_temp(egui::Id::new("board_status_height"),status_panel.response.rect.height()));
  self.ui_dashboard(ctx);
  tool_window::fit_content(ctx,Tab::Dashboard);
  let mut closed=vec![];
  for tab in self.tool_windows.clone() {
   let id=egui::ViewportId::from_hash_of(format!("tool-{tab:?}"));
   let d=tool_window::design(tab);let (size,min)=d.geometry(ctx.input(|i|i.viewport().monitor_size));
   ctx.show_viewport_immediate(id,egui::ViewportBuilder::default().with_title(format!("{} — SetupScope",setupscope_core::i18n::t(tab.title()))).with_taskbar(true).with_inner_size(size).with_min_inner_size(min),|ctx,_| {
    if ctx.input(|i|i.viewport().close_requested()){closed.push(tab);}
    self.render_tool(ctx,tab);
    super::a11y::focus_ring(ctx,theme::ACCENT_TEXT);
   });
  }
  self.close_tools(ctx,&closed);
  super::a11y::focus_ring(ctx,theme::ACCENT_TEXT);
 }
}
#[cfg(test)] mod tests {
 #[test] fn localized_controls_fit_compact_windows() {
  // Each locale runs in its own process: global language changes must never
  // race tests that assert original English labels or diagnostic evidence.
  let Ok(locale)=std::env::var("SETUPSCOPE_LAYOUT_LOCALE") else {
   for locale in ["en","de","es","fr","it","ja","ko","pl","pt-BR","ru","zh-CN"] {
    let status=std::process::Command::new(std::env::current_exe().unwrap())
     .args(["--exact","gui::app::tests::localized_controls_fit_compact_windows","--nocapture","--test-threads=1"])
     .env("SETUPSCOPE_LAYOUT_LOCALE",locale).status().unwrap();
    assert!(status.success(),"{locale}: localized layout failed");
   } return;
  };
  setupscope_core::i18n::set_language(&locale);
  let root=std::env::temp_dir().join(format!("localized-layout-{}",setupscope_core::util::new_id()));
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  for tab in NAVIGATION.iter().map(|(tab,_,_,_)|*tab).chain([Tab::Doctor,Tab::Developer,Tab::Bundle,Tab::Dependencies,Tab::Settings]) {
   app.readiness_page=tab;
   for width in [tool_window::design(tab).minimum[0],tool_window::design(tab).size[0]] {
    let ctx=egui::Context::default();ctx.enable_accesskit();super::super::install_fonts(&ctx);theme::apply(&ctx,false);
    let mut last=None;
    for _ in 0..3 {last=Some(ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,800.))),..Default::default()},|ctx|app.render_tool(ctx,tab.owner())));}
    for (_,node) in last.unwrap().platform_output.accesskit_update.unwrap().nodes {
     if matches!(format!("{:?}",node.role()).as_str(),"Button"|"TextInput"|"ComboBox") {
      if let Some(b)=node.bounds(){assert!(b.x0>=-1. && b.x1<=width as f64+1.,"{locale} {tab:?}: {:?} outside {width}px: {b:?}",node.role());}
     }
    }
   }
  }
  drop(app);std::fs::remove_dir_all(root).unwrap();
 }
 use super::*;
 #[test] fn board_has_exactly_five_unique_workflows() {
  assert_eq!(NAVIGATION.len(),5);
  let mut seen=vec![];for &(t,_,_,_) in NAVIGATION {assert_eq!(t,t.owner());assert!(!seen.contains(&t));seen.push(t);}
  assert!(!seen.contains(&Tab::Settings));
 }
 #[test] fn dashboard_has_three_columns_and_two_rows_with_system_card(){
  let root=std::env::temp_dir().join(format!("board-grid-{}",setupscope_core::util::new_id()));
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  for width in [740.,960.] {
   let ctx=egui::Context::default();theme::apply(&ctx,false);let mut output=None;
   for _ in 0..2 {output=Some(ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,900.))),..Default::default()},|ctx|app.ui_dashboard(ctx)));}
   let output=output.unwrap();let mut positions=vec![];
   for title in NAVIGATION.iter().map(|(_,_,s,_)|*s).chain(["System And OS"]){
    let t=output.shapes.iter().find_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape{(t.galley.text()==title).then_some(t)}else{None}).unwrap_or_else(||panic!("Missing {title} at {width}: {:?}",output.shapes.iter().filter_map(|s|if let egui::epaint::Shape::Text(t)=&s.shape{Some((t.galley.text(),t.pos))}else{None}).collect::<Vec<_>>()));
    assert!(t.pos.x>=0. && t.pos.x+t.galley.size().x<=width+1.);positions.push(t.pos);
   }
   assert!(positions[0].x<positions[1].x && positions[1].x<positions[2].x);
   assert!((positions[0].y-positions[2].y).abs()<2.);
   assert!(positions[3].y>positions[0].y+100. && (positions[3].y-positions[5].y).abs()<12.);
   assert!(app.tool_windows.is_empty(),"Rendering the system card must not open a tool");
  }
  drop(app);std::fs::remove_dir_all(root).unwrap();
 }
 #[test] fn repeated_open_reuses_tool_and_detail_window() {
  let root=std::env::temp_dir().join(format!("tool-singleton-{}",setupscope_core::util::new_id()));
  let ctx=egui::Context::default();
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  for &(t,_,_,_) in NAVIGATION {app.open_tool(&ctx,t);app.open_tool(&ctx,t);}
  app.open_tool(&ctx,Tab::Doctor);app.open_tool(&ctx,Tab::Developer);app.open_tool(&ctx,Tab::Bundle);
  assert_eq!(app.tool_windows.len(),5);
  assert_eq!(app.tool_windows.iter().filter(|t|**t==Tab::ProjectReadiness).count(),1);
  app.tool_windows.retain(|t|*t!=Tab::Ports);app.open_tool(&ctx,Tab::Ports);
  assert_eq!(app.tool_windows.len(),5);
  std::fs::remove_dir_all(root).unwrap();
 }
 #[test] fn compact_tool_controls_fit_minimum_window_widths() {
  let root=std::env::temp_dir().join(format!("tool-widths-{}",setupscope_core::util::new_id()));
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  for tab in NAVIGATION.iter().map(|(tab,_,_,_)|*tab).chain([Tab::Doctor,Tab::Developer,Tab::Bundle,Tab::Dependencies,Tab::Settings]) {
   app.readiness_page=tab;
   let ctx=egui::Context::default();ctx.enable_accesskit();theme::apply(&ctx,false);
   let width=tool_window::design(tab).minimum[0];
   let mut last=None;
   for _ in 0..2 {
    last=Some(ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,600.))),..Default::default()},|ctx|app.render_tool(ctx,tab.owner())));
   }
   let update=last.unwrap().platform_output.accesskit_update.unwrap();
   for (_,node) in update.nodes {
    if matches!(format!("{:?}",node.role()).as_str(),"Button"|"TextInput"|"ComboBox") {
     if let Some(bounds)=node.bounds() {assert!(bounds.x0>=-1. && bounds.x1<=width as f64+1.,"{tab:?}: {:?} is outside {width}px: {bounds:?}",node.role());}
    }
   }
  }
  drop(app);std::fs::remove_dir_all(root).unwrap();
 }
 #[test] fn focusing_readiness_preserves_dependency_review_context() {
  let root=std::env::temp_dir().join(format!("dependency-focus-{}",setupscope_core::util::new_id()));
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  let ctx=egui::Context::default();app.open_tool(&ctx,Tab::Dependencies);app.open_tool(&ctx,Tab::ProjectReadiness);
  assert_eq!(app.readiness_page,Tab::Dependencies);assert_eq!(app.tool_windows,vec![Tab::ProjectReadiness]);
  drop(app);std::fs::remove_dir_all(root).unwrap();
 }
 #[test] fn diagnostic_details_share_readiness_window() {for t in [Tab::Doctor,Tab::Developer,Tab::Bundle,Tab::Dependencies]{assert_eq!(t.owner(),Tab::ProjectReadiness);}}
 #[test] fn board_returns_without_closing_tools_and_last_close_restores_it() {
  let root=std::env::temp_dir().join(format!("board-navigation-{}",setupscope_core::util::new_id()));
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  let ctx=egui::Context::default();
  app.open_tool(&ctx,Tab::ProjectReadiness);assert!(app.board_hidden);
  app.show_board(&ctx);assert!(!app.board_hidden);assert_eq!(app.tool_windows,vec![Tab::ProjectReadiness]);
  app.open_tool(&ctx,Tab::BuildLog);assert!(app.board_hidden);
  app.show_board(&ctx);assert_eq!(app.tool_windows,vec![Tab::ProjectReadiness,Tab::BuildLog]);
  app.open_tool(&ctx,Tab::BuildLog);assert_eq!(app.tool_windows.len(),2);
  app.close_tools(&ctx,&[Tab::BuildLog]);assert!(app.board_hidden);assert_eq!(app.tool_windows,vec![Tab::ProjectReadiness]);
  app.close_tools(&ctx,&[Tab::ProjectReadiness]);assert!(!app.board_hidden&&app.tool_windows.is_empty());
  drop(app);std::fs::remove_dir_all(root).unwrap();
 }
 #[test] fn main_header_names_the_toolkit_and_settings_stays_at_right() {
  let root=std::env::temp_dir().join(format!("board-header-{}",setupscope_core::util::new_id()));
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  for width in [440.,600.] {
   let ctx=egui::Context::default();theme::apply(&ctx,false);let mut output=None;
   for _ in 0..2 {output=Some(ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,630.))),..Default::default()},|ctx|app.ui_main_header(ctx)));}
   let output=output.unwrap();let mut settings=false;let mut title=false;
   for clipped in &output.shapes {if let egui::epaint::Shape::Text(t)=&clipped.shape {
    if t.galley.text()=="Settings" {settings=true;assert!(t.pos.x>width-110. && t.pos.x+t.galley.size().x<=width);}
    if t.galley.text().contains("Windows Software Development Toolkit"){title=true;}
   }}assert!(settings&&title);
  }
  drop(app);std::fs::remove_dir_all(root).unwrap();
 }
 #[test] fn alternating_tool_headers_do_not_clip_readiness_navigation() {
  let root=std::env::temp_dir().join(format!("tool-headers-{}",setupscope_core::util::new_id()));
  let mut app=App{store:Store::at(&root).unwrap(),settings:Settings::default(),devcheck:Default::default(),readiness:Default::default(),tool_windows:vec![],readiness_page:Tab::ProjectReadiness,status:String::new(),system_info:Default::default(),board_hidden:false};
  let ctx=egui::Context::default();theme::apply(&ctx,false);
  for i in 0..8 {
   let tab=if i%2==0 {Tab::BuildLog}else{Tab::ProjectReadiness};
   let output=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(760.,600.))),..Default::default()},|ctx|app.render_tool(ctx,tab));
   if i>=3 && tab==Tab::ProjectReadiness {
    let text=output.shapes.iter().find_map(|clipped|if let egui::epaint::Shape::Text(text)=&clipped.shape { (text.galley.text()=="Dependencies").then_some((clipped,text)) } else {None}).expect("Dependencies navigation must be painted");
    assert!(text.0.clip_rect.contains_rect(egui::Rect::from_min_size(text.1.pos,text.1.galley.size())),"Another tool's panel state clipped readiness navigation");
   }
  }
  drop(app);std::fs::remove_dir_all(root).unwrap();
 }
}
