use anyhow::{anyhow, Result};
use aws_config::BehaviorVersion;
use aws_sdk_ecs::Client;
use clap::Parser;
use dialoguer::Select;
use std::process::{Command, Stdio};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "ECS Execute Command utility for connecting to running tasks",
    after_help = "Example usage:\n    AWS_PROFILE=uat-admin ecs_remote -t {container-name} -p uat-admin"
)]
struct Args {
    /// AWS Profile name to use
    #[arg(short = 'p', long, default_value = "default")]
    profile: String,

    /// Target cluster name or ARN
    #[arg(short = 'l', long)]
    cluster: Option<String>,

    /// Target service name
    #[arg(short = 's', long)]
    service: Option<String>,

    /// Container name to execute command in
    #[arg(short = 't', long)]
    container: String,
}

#[derive(Debug, Clone)]
struct TaskInfo {
    arn: String,
    task_id: String,
    task_name: String,
}

#[derive(Debug, Clone)]
struct ServiceInfo {
    arn: String,
    service_name: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let config = aws_config::from_env()
        .behavior_version(BehaviorVersion::v2024_03_28())
        .profile_name(&args.profile)
        .credentials_provider(
            aws_config::default_provider::credentials::Builder::default()
                .profile_name(&args.profile)
                .build()
                .await,
        )
        .load()
        .await;

    let ecs_client = Client::new(&config);

    // 1. List clusters and select one
    let clusters = list_clusters(&ecs_client).await?;
    if clusters.is_empty() {
        return Err(anyhow!("No clusters found."));
    }

    let cluster_arn = match args.cluster {
        Some(ref cluster) => {
            // Find the matching cluster ARN
            clusters
                .iter()
                .find(|arn| arn.contains(cluster))
                .ok_or_else(|| anyhow!("Specified cluster '{}' not found", cluster))?
                .clone()
        }
        None => select_cluster(clusters)?,
    };

    // 2. List and select services in the cluster
    let services = list_services(&ecs_client, &cluster_arn).await?;
    if services.is_empty() {
        return Err(anyhow!("No services found in cluster {}", cluster_arn));
    }

    let service = match args.service {
        Some(ref service_name) => services
            .iter()
            .find(|s| s.service_name == *service_name)
            .ok_or_else(|| anyhow!("Specified service '{}' not found", service_name))?
            .clone(),
        None => select_service(services)?,
    };

    // 3. List and validate tasks in the selected service
    let tasks = list_valid_tasks(&ecs_client, &cluster_arn, &service.service_name).await?;
    if tasks.is_empty() {
        return Err(anyhow!(
            "No tasks with execute command enabled found in service {}",
            service.service_name
        ));
    }

    let task = select_task(tasks)?;

    // 4. Execute the AWS CLI execute-command to open an interactive shell
    execute_shell(&cluster_arn, &task.arn, &args.container, &args.profile)?;
    Ok(())
}

// List available clusters
async fn list_clusters(client: &Client) -> Result<Vec<String>> {
    let mut cluster_arns = Vec::new();
    let mut next_token = None;

    loop {
        let mut request = client.list_clusters();
        if let Some(token) = next_token {
            request = request.next_token(token);
        }

        let response = request.send().await?;
        if let Some(arns) = response.cluster_arns {
            cluster_arns.extend(arns);
        }

        match response.next_token {
            Some(token) => next_token = Some(token),
            None => break,
        }
    }

    Ok(cluster_arns)
}

// List services in a cluster
async fn list_services(client: &Client, cluster_arn: &str) -> Result<Vec<ServiceInfo>> {
    let mut services = Vec::new();
    let mut next_token = None;

    loop {
        let mut request = client.list_services().cluster(cluster_arn);
        if let Some(token) = next_token {
            request = request.next_token(token);
        }

        let response = request.send().await?;
        if let Some(service_arns) = response.service_arns {
            for arn in service_arns {
                let service_name = arn.split('/').last().unwrap_or(&arn).to_string();
                services.push(ServiceInfo { arn, service_name });
            }
        }

        match response.next_token {
            Some(token) => next_token = Some(token),
            None => break,
        }
    }

    services.sort_by(|a, b| a.service_name.cmp(&b.service_name));
    Ok(services)
}

// List only valid tasks in a given service
async fn list_valid_tasks(
    client: &Client,
    cluster_arn: &str,
    service_name: &str,
) -> Result<Vec<TaskInfo>> {
    let mut valid_tasks = Vec::new();
    let mut next_token = None;

    loop {
        let mut request = client
            .list_tasks()
            .cluster(cluster_arn)
            .service_name(service_name)
            .desired_status("RUNNING".into());

        if let Some(token) = next_token {
            request = request.next_token(token);
        }

        let response = request.send().await?;

        if let Some(task_arns) = response.task_arns {
            // If we have tasks, describe them to validate their status
            if !task_arns.is_empty() {
                let desc_response = client
                    .describe_tasks()
                    .cluster(cluster_arn)
                    .set_tasks(Some(task_arns))
                    .send()
                    .await?;

                if let Some(tasks) = desc_response.tasks {
                    for task in tasks {
                        // Only include tasks that are actually running and have execute command enabled
                        if task.last_status == Some("RUNNING".to_string())
                            && task.enable_execute_command == true
                        {
                            if let (Some(arn), Some(task_def)) =
                                (task.task_arn.clone(), task.task_definition_arn)
                            {
                                // Get task definition details to get the task family name
                                let def_response = client
                                    .describe_task_definition()
                                    .task_definition(task_def)
                                    .send()
                                    .await?;

                                if let Some(task_def) = def_response.task_definition {
                                    let task_id = arn.split('/').last().unwrap_or(&arn).to_string();
                                    let family_name =
                                        task_def.family.unwrap_or_else(|| "unknown".to_string());

                                    valid_tasks.push(TaskInfo {
                                        arn,
                                        task_id,
                                        task_name: family_name,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        match response.next_token {
            Some(token) => next_token = Some(token),
            None => break,
        }
    }

    valid_tasks.sort_by(|a, b| a.task_name.cmp(&b.task_name));
    Ok(valid_tasks)
}

// Interactive helper to select a cluster
fn select_cluster(clusters: Vec<String>) -> Result<String> {
    let display_clusters: Vec<String> = clusters
        .iter()
        .map(|arn| arn.split('/').last().unwrap_or(arn).to_string())
        .collect();

    let selection = Select::new()
        .with_prompt("Select Cluster")
        .items(&display_clusters)
        .default(0)
        .interact()?;

    Ok(clusters[selection].clone())
}

// Interactive helper to select a service
fn select_service(services: Vec<ServiceInfo>) -> Result<ServiceInfo> {
    let display_services: Vec<String> = services
        .iter()
        .map(|service| service.service_name.clone())
        .collect();

    let selection = Select::new()
        .with_prompt("Select Service")
        .items(&display_services)
        .default(0)
        .interact()?;

    Ok(services[selection].clone())
}

// Interactive helper to select a task
fn select_task(tasks: Vec<TaskInfo>) -> Result<TaskInfo> {
    let display_tasks: Vec<String> = tasks
        .iter()
        .map(|task| format!("{} ({})", task.task_name, task.task_id))
        .collect();

    let selection = Select::new()
        .with_prompt("Select Task for ECS Exec")
        .items(&display_tasks)
        .default(0)
        .interact()?;

    Ok(tasks[selection].clone())
}

// Execute the AWS CLI execute-command to open an interactive shell
fn execute_shell(cluster_arn: &str, task_arn: &str, container: &str, profile: &str) -> Result<()> {
    // Extract the cluster name and task ID from the ARNs
    let cluster_name = cluster_arn.split('/').last().unwrap_or(cluster_arn);
    let task_id = task_arn.split('/').last().unwrap_or(task_arn);

    Command::new("aws")
        .args([
            "ecs",
            "execute-command",
            "--cluster",
            cluster_name,
            "--task",
            task_id,
            "--container",
            container,
            "--command",
            "/bin/bash",
            "--interactive",
            "--profile",
            profile,
        ])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?
        .wait()?;

    Ok(())
}

