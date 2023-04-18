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
struct UserInfo {
  id: usize,
  name: String,
  username: String,
}

fn user_info(client: &Client, user: &str) -> Result<UserInfo> {
  let response: Vec<UserInfo> = client
    .get("https://gitlab.com/api/v4/users")
    .query(&[("username", user)])
    .send()?
    .json()?;

  if response.is_empty() {
    return Err("No user found with that name".into());
  }

  Ok(response.into_iter().last().unwrap())
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

fn recent_pushes(client: &Client, user: &UserInfo) -> Result<Vec<RecentPush>> {
  let pushes = client
    .get(format!(
      "https://gitlab.com/api/v4/users/{}/events",
      user.id
    ))
    .query(&[("action", "pushed")])
    .send()?
    .json()?;
  Ok(pushes)
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
  author: UserInfo,
  assignees: Vec<UserInfo>,
  reviewers: Vec<UserInfo>,
}

fn reviewing(client: &Client, user: &str) -> Result<HashMap<usize, MergeRequest>> {
  let mrs: Vec<MergeRequest> = client
    .get("https://gitlab.com/api/v4/merge_requests")
    .query(&[
      ("state", "opened"),
      ("scope", "all"),
      ("reviewer_username", user),
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

fn assigned(client: &Client, user: &str) -> Result<HashMap<usize, MergeRequest>> {
  let mrs: Vec<MergeRequest> = client
    .get("https://gitlab.com/api/v4/merge_requests")
    .query(&[
      ("state", "opened"),
      ("scope", "all"),
      ("assignee_username", user),
    ])
    .send()?
    .json()?;
  let mrs = mrs.into_iter().map(|mr| (mr.id, mr)).collect();
  Ok(mrs)
}
fn authored(client: &Client, user: &str) -> Result<HashMap<usize, MergeRequest>> {
  let mrs: Vec<MergeRequest> = client
    .get("https://gitlab.com/api/v4/merge_requests")
    .query(&[
      ("state", "opened"),
      ("scope", "all"),
      ("author_username", user),
    ])
    .send()?
    .json()?;
  let mrs = mrs.into_iter().map(|mr| (mr.id, mr)).collect();
  Ok(mrs)
}

fn mrs_for_branch(
  client: &Client,
  project_id: usize,
  branch: &str,
) -> Result<HashMap<usize, MergeRequest>> {
  let mrs: Vec<MergeRequest> = client
    .get(format!(
      "https://gitlab.com/api/v4/projects/{}/merge_requests",
      project_id
    ))
    .query(&[
      ("state", "opened"),
      ("scope", "all"),
      ("source_branch", branch),
    ])
    .send()?
    .json()?;
  let mrs: HashMap<usize, MergeRequest> = mrs.into_iter().map(|mr| (mr.id, mr)).collect();
  Ok(mrs)
}

fn all_mrs(client: &Client, user: &UserInfo) -> Result<HashMap<usize, MergeRequest>> {
  let recent_mrs: HashMap<usize, MergeRequest> = recent_pushes(&client, &user)?
    .iter()
    .map(|recent_push| {
      mrs_for_branch(
        &client,
        recent_push.project_id,
        recent_push.push_data.ref_.as_str(),
      )
    })
    .collect::<Result<Vec<_>>>()?
    .into_iter()
    .flat_map(|mrs| mrs.into_iter())
    .collect();
  let to_review = reviewing(&client, user.username.as_str())?;
  let assigned = assigned(&client, user.username.as_str())?;
  let authored = authored(&client, user.username.as_str())?;

  let all_mrs: HashMap<usize, MergeRequest> = recent_mrs
    .into_iter()
    .chain(to_review)
    .chain(assigned)
    .chain(authored)
    .collect();

  Ok(all_mrs)
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct Approver {
  user: UserInfo,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalInfo {
  approvals_left: usize,
  approved_by: Vec<Approver>,
}

fn approval_info(client: &Client, mr: &MergeRequest) -> Result<ApprovalInfo> {
  let info = client
    .get(format!(
      "https://gitlab.com/api/v4/projects/{}/merge_requests/{}/approvals",
      mr.project_id, mr.iid
    ))
    .send()?
    .json()?;
  Ok(info)
}

fn make_link(url: &str, title: &str) -> String {
  format!("\x1B]8;;{}\x1B\\{}\x1B]8;;\x1B\\", url, title)
}

fn priority(mr: &MergeRequest, approval_info: &ApprovalInfo, user: &UserInfo) -> isize {
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

fn print_all(client: &Client, user_info: &UserInfo) -> Result<()> {
  let all_mrs: HashMap<usize, MergeRequest> = all_mrs(&client, &user_info)?;
  let mut all_mrs: Vec<(MergeRequest, ApprovalInfo)> = all_mrs
    .into_iter()
    .map(|(_, mr)| approval_info(&client, &mr).map(|approval_info| (mr, approval_info)))
    .collect::<Result<_>>()?;

  all_mrs.sort_by(|lhs, rhs| {
    let lhs_prio = priority(&lhs.0, &lhs.1, &user_info);
    let rhs_prio = priority(&rhs.0, &rhs.1, &user_info);
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
      if mr
        .assignees
        .iter()
        .any(|assignee| assignee.id == user_info.id)
      {
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

  let user_info = user_info(&client, user_name.as_str())?;

  loop {
    print_all(&client, &user_info)?;
    sleep(Duration::from_secs(30));
  }
}
