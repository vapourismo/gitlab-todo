use chrono::{DateTime, Utc};
use crossterm::{
  style::{Color, Print, Stylize},
  terminal::{Clear, ClearType},
};
use reqwest::{blocking::Client, header::HeaderMap};
use serde::Deserialize;
use std::{
  collections::HashMap,
  env,
  error::Error,
  io::{stdout, Write},
  ops::Sub,
  thread::sleep,
  time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct User {
  id: usize,
  name: String,
  username: String,
}

impl User {
  fn get<UserName: AsRef<str>>(client: &Client, user: UserName) -> Result<Self> {
    let response: Vec<User> = client
      .get("https://gitlab.com/api/v4/users")
      .query(&[("username", user.as_ref())])
      .send()?
      .json()?;

    response
      .into_iter()
      .last()
      .ok_or("No user found with that name".into())
  }

  fn get_recent_pushes(&self, client: &Client) -> Result<Vec<RecentPush>> {
    Ok(
      client
        .get(format!(
          "https://gitlab.com/api/v4/users/{}/events",
          self.id
        ))
        .query(&[("action", "pushed")])
        .send()?
        .json()?,
    )
  }

  fn get_mrs_to_review(&self, client: &Client) -> Result<HashMap<usize, MergeRequest>> {
    let mrs: Vec<MergeRequest> = client
      .get("https://gitlab.com/api/v4/merge_requests")
      .query(&[
        ("state", "opened"),
        ("scope", "all"),
        ("reviewer_username", self.username.as_str()),
      ])
      .send()?
      .json()?;
    let now = Utc::now();
    let mrs = mrs
      .into_iter()
      .filter(|mr| now.sub(mr.updated_at).num_days() <= 14)
      .map(|mr| (mr.id, mr))
      .collect();
    Ok(mrs)
  }

  fn get_assigned_mrs(&self, client: &Client) -> Result<HashMap<usize, MergeRequest>> {
    let mrs: Vec<MergeRequest> = client
      .get("https://gitlab.com/api/v4/merge_requests")
      .query(&[
        ("state", "opened"),
        ("scope", "all"),
        ("assignee_username", self.username.as_str()),
      ])
      .send()?
      .json()?;
    let mrs = mrs.into_iter().map(|mr| (mr.id, mr)).collect();
    Ok(mrs)
  }

  fn get_authored_mrs(&self, client: &Client) -> Result<HashMap<usize, MergeRequest>> {
    let mrs: Vec<MergeRequest> = client
      .get("https://gitlab.com/api/v4/merge_requests")
      .query(&[
        ("state", "opened"),
        ("scope", "all"),
        ("author_username", self.username.as_str()),
      ])
      .send()?
      .json()?;
    let mrs = mrs.into_iter().map(|mr| (mr.id, mr)).collect();
    Ok(mrs)
  }

  fn get_related_mrs(&self, client: &Client) -> Result<HashMap<usize, MergeRequest>> {
    let recent_mrs: HashMap<usize, MergeRequest> = self
      .get_recent_pushes(&client)?
      .iter()
      .map(|recent_push| {
        MergeRequest::get_by_branch(&client, recent_push.project_id, &recent_push.push_data.ref_)
      })
      .collect::<Result<Vec<_>>>()?
      .into_iter()
      .flat_map(|mrs| mrs.into_iter())
      .collect();
    let to_review = self.get_mrs_to_review(&client)?;
    let assigned = self.get_assigned_mrs(&client)?;
    let authored = self.get_authored_mrs(&client)?;

    let all_mrs: HashMap<usize, MergeRequest> = recent_mrs
      .into_iter()
      .chain(to_review)
      .chain(assigned)
      .chain(authored)
      .collect();

    Ok(all_mrs)
  }
}

#[derive(Deserialize, Debug, Clone)]
struct PushData {
  #[serde(rename = "ref")]
  ref_: String,
}

#[derive(Deserialize, Debug, Clone)]
struct RecentPush {
  project_id: usize,
  push_data: PushData,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct References {
  full: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct Milestone {
  title: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct MergeRequest {
  id: usize,
  iid: usize,
  project_id: usize,
  title: String,
  milestone: Option<Milestone>,
  draft: bool,
  has_conflicts: bool,
  references: References,
  web_url: String,
  updated_at: DateTime<Utc>,
  author: User,
  assignees: Vec<User>,
  reviewers: Vec<User>,
}

impl MergeRequest {
  fn get_by_branch<BranchName: AsRef<str>>(
    client: &Client,
    project_id: usize,
    branch: BranchName,
  ) -> Result<HashMap<usize, MergeRequest>> {
    let mrs: Vec<MergeRequest> = client
      .get(format!(
        "https://gitlab.com/api/v4/projects/{}/merge_requests",
        project_id
      ))
      .query(&[
        ("state", "opened"),
        ("scope", "all"),
        ("source_branch", branch.as_ref()),
      ])
      .send()?
      .json()?;
    let mrs: HashMap<usize, MergeRequest> = mrs.into_iter().map(|mr| (mr.id, mr)).collect();
    Ok(mrs)
  }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct Approver {
  user: User,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalInfo {
  approvals_left: usize,
  approved_by: Vec<Approver>,
}

impl ApprovalInfo {
  fn get(client: &Client, mr: &MergeRequest) -> Result<Self> {
    let info = client
      .get(format!(
        "https://gitlab.com/api/v4/projects/{}/merge_requests/{}/approvals",
        mr.project_id, mr.iid
      ))
      .send()?
      .json()?;
    Ok(info)
  }
}

fn make_link(url: &str, title: &str) -> String {
  format!("\x1B]8;;{}\x1B\\{}\x1B]8;;\x1B\\", url, title)
}

fn priority(mr: &MergeRequest, approval_info: &ApprovalInfo, user: &User) -> isize {
  let mut prio = 0;

  if mr.assignees.iter().any(|assignee| assignee.id == user.id) {
    prio += 5;
  }

  if mr.author.id == user.id {
    prio += 1;
  }

  if mr.reviewers.iter().any(|reviewer| reviewer.id == user.id) {
    prio += 1;
  }

  if mr.has_conflicts {
    prio -= 1;
  }

  if approval_info.approvals_left < 1 {
    prio -= 2;
  }

  if mr
    .assignees
    .iter()
    .all(|assignee| assignee.username == "nomadic-margebot")
  {
    prio -= 5;
  }

  prio
}

fn cell(width: usize, body: &str) -> String {
  let len = body.len();

  if len > width {
    let mut body: String = body.chars().take(width - 3).collect();
    body.push_str("...");
    body
  } else {
    let suffix = " ".repeat(width - len);
    let mut body = body.to_string();
    body.push_str(&suffix);
    body
  }
}

fn print_all(client: &Client, user: &User) -> Result<()> {
  let all_mrs: HashMap<usize, MergeRequest> = user.get_related_mrs(&client)?;
  let mut all_mrs: Vec<(MergeRequest, ApprovalInfo)> = all_mrs
    .into_iter()
    .map(|(_, mr)| ApprovalInfo::get(&client, &mr).map(|approval_info| (mr, approval_info)))
    .collect::<Result<_>>()?;

  all_mrs.sort_by(|lhs, rhs| {
    let lhs_prio = priority(&lhs.0, &lhs.1, &user);
    let rhs_prio = priority(&rhs.0, &rhs.1, &user);
    lhs_prio.cmp(&rhs_prio).reverse()
  });

  let mut target = stdout();
  let ref_width = all_mrs
    .iter()
    .map(|(mr, _)| mr.references.full.len())
    .max()
    .unwrap_or(25);
  let assignee_width = 15;
  let reviewer_width = 40;
  let title_width = all_mrs
    .iter()
    .map(|(mr, _)| mr.title.len())
    .max()
    .unwrap_or(80);

  crossterm::execute!(target, Clear(ClearType::All))?;
  for (mr, approval_info) in all_mrs {
    let reference = make_link(&mr.web_url, &cell(ref_width, &mr.references.full)).blue();
    let title = cell(title_width, &mr.title).with(
      if mr.assignees.iter().any(|assignee| assignee.id == user.id) {
        Color::Red
      } else if approval_info.approvals_left < 1 {
        Color::Green
      } else if mr.draft {
        Color::Grey
      } else {
        Color::White
      },
    );
    let assignees = cell(
      assignee_width,
      mr.assignees
        .iter()
        .map(|a| format!("{} ", a.username))
        .collect::<String>()
        .as_str(),
    )
    .red();
    let reviewers = cell(
      reviewer_width,
      mr.reviewers
        .iter()
        .map(|r| format!("{} ", r.username))
        .collect::<String>()
        .as_str(),
    )
    .grey();

    crossterm::execute!(
      target,
      Print(reference),
      Print(" "),
      Print(title),
      Print(" "),
      Print(assignees),
      Print(" "),
      Print(reviewers)
    )?;
    writeln!(target, "")?;
  }

  Ok(())
}

fn main() -> Result<()> {
  let gitlab_token = env::var("GITLAB_TOKEN")?;

  let client = Client::builder()
    .default_headers(HeaderMap::from_iter([(
      "Authorization".parse().unwrap(),
      format!("Bearer {}", gitlab_token).parse()?,
    )]))
    .build()?;

  let user_name = env::args()
    .nth(1)
    .ok_or::<Box<dyn Error>>("First argument should be the GitLab user name".into())?;

  let user = User::get(&client, user_name.as_str())?;

  loop {
    print_all(&client, &user)?;
    sleep(Duration::from_secs(30));
  }
}
