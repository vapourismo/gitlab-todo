use chrono::{DateTime, Utc};
use reqwest::{blocking::Client, header::HeaderMap};
use serde::Deserialize;
use std::{collections::HashMap, env, error::Error, ops::Sub};

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
  dbg!(&user_name);

  let user_info = user_info(&client, user_name.as_str())?;
  dbg!(&user_info);

  let all_mrs: HashMap<usize, MergeRequest> = all_mrs(&client, &user_info)?;
  let all_mrs: Vec<(MergeRequest, ApprovalInfo)> = all_mrs
    .into_iter()
    .map(|(_, mr)| approval_info(&client, &mr).map(|approval_info| (mr, approval_info)))
    .collect::<Result<_>>()?;

  dbg!(&all_mrs);

  Ok(())
}
