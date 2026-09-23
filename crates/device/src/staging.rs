//! Scoped native access to the single transaction-owned staging link.
//! Cleanup treats generated symlinks as leaves and never follows them.
use super::*;
use aircard_core::staging::StagingPlan;

fn invalid(stage: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, stage)
}
fn field<'a>(info: &'a [String], key: &str) -> Option<&'a str> {
    info.as_chunks::<2>()
        .0
        .iter()
        .find(|p| p[0] == key)
        .map(|p| p[1].as_str())
}
impl LinuxAfc {
    pub fn staging_preflight(&mut self, plan: &StagingPlan) -> Result<()> {
        plan.validate().map_err(|_| invalid("staging_plan"))?;
        for path in [
            plan.source(),
            plan.link(),
            format!("AirCard-Linux-Work-{}", plan.transaction),
        ] {
            match self.0.info(&path) {
                Err(e) if e.domain == 4 && e.code == 8 => {}
                Err(e) => return Err(Error::native(e, "staging_collision_check")),
                Ok(_) => return Err(Error::new(ErrorKind::Conflict, "staging_collision")),
            }
        }
        Ok(())
    }
    pub fn customization_staged(&mut self, plan: &aircard_core::customization::Plan) -> Result<()> {
        plan.validate().map_err(|_| invalid("customization_plan"))?;
        let info = self
            .0
            .info(&format!("{}/p0/p1/p2/link", plan.source()))
            .map_err(|e| Error::native(e, "customization_link_stat"))?;
        let expected = format!("../../..{}", plan.target.directory());
        if field(&info, "st_ifmt") != Some("S_IFLNK")
            || field(&info, "LinkTarget") != Some(expected.as_str())
        {
            return Err(invalid("customization_link_target"));
        }
        Ok(())
    }
    /// Removes only generated roots, treating symlinks as leaves. Never follows them on cleanup.
    pub fn staging_cleanup(&mut self, plan: &StagingPlan) -> Result<()> {
        plan.validate().map_err(|_| invalid("staging_plan"))?;
        for root in [
            plan.link(),
            plan.source(),
            format!("AirCard-Linux-Work-{}", plan.transaction),
        ] {
            let mut pending = vec![root.clone()];
            let mut paths = vec![];
            while let Some(path) = pending.pop() {
                if paths.len() + pending.len() >= 1024 || path.split('/').count() > 20 {
                    return Err(invalid("staging_cleanup_limit"));
                }
                let info = match self.0.info(&path) {
                    Err(e) if e.domain == 4 && e.code == 8 => continue,
                    Err(e) => return Err(Error::native(e, "staging_cleanup_stat")),
                    Ok(info) => info,
                };
                match field(&info, "st_ifmt") {
                    Some("S_IFDIR") => {
                        // A moved link is never traversed, even if replaced by a directory concurrently.
                        if root == plan.link() {
                            return Err(Error::new(ErrorKind::Conflict, "staging_link_replaced"));
                        }
                        for leaf in self
                            .0
                            .list(&path)
                            .map_err(|e| Error::native(e, "staging_cleanup_list"))?
                        {
                            if leaf == "." || leaf == ".." {
                                continue;
                            }
                            aircard_core::safe_leaf(&leaf)
                                .map_err(|_| invalid("staging_cleanup_leaf"))?;
                            if pending.len() + paths.len() >= 1024 {
                                return Err(invalid("staging_cleanup_limit"));
                            }
                            pending.push(format!("{path}/{leaf}"));
                        }
                    }
                    Some("S_IFREG" | "S_IFLNK") => {}
                    _ => return Err(invalid("staging_cleanup_type")),
                }
                paths.push(path);
            }
            paths.sort_by_key(|p| std::cmp::Reverse(p.split('/').count()));
            for path in paths {
                self.0
                    .remove(&path)
                    .map_err(|e| Error::native(e, "staging_cleanup_remove"))?;
            }
        }
        self.staging_preflight(plan)
    }
}
