use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct StructurizrWorkspaceJson {
    pub(super) views: Option<StructurizrViewsJson>,
    pub(super) model: Option<StructurizrModelJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StructurizrModelJson {
    pub(super) people: Option<Vec<StructurizrElementJson>>,
    pub(super) software_systems: Option<Vec<StructurizrSystemJson>>,
    pub(super) deployment_nodes: Option<Vec<StructurizrNodeJson>>,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct StructurizrElementJson {
    pub(super) id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) technology: Option<String>,
    pub(super) tags: Option<String>,
    pub(super) relationships: Option<Vec<StructurizrRelationshipJson>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct StructurizrSystemJson {
    pub(super) id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) tags: Option<String>,
    pub(super) containers: Option<Vec<StructurizrContainerJson>>,
    pub(super) relationships: Option<Vec<StructurizrRelationshipJson>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct StructurizrContainerJson {
    pub(super) id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) technology: Option<String>,
    pub(super) tags: Option<String>,
    pub(super) components: Option<Vec<StructurizrElementJson>>,
    pub(super) relationships: Option<Vec<StructurizrRelationshipJson>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct StructurizrNodeJson {
    pub(super) id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) technology: Option<String>,
    pub(super) tags: Option<String>,
    pub(super) children: Option<Vec<StructurizrNodeJson>>,
    pub(super) infrastructure_nodes: Option<Vec<StructurizrElementJson>>,
    pub(super) software_system_instances: Option<Vec<StructurizrElementJson>>,
    pub(super) container_instances: Option<Vec<StructurizrElementJson>>,
    pub(super) relationships: Option<Vec<StructurizrRelationshipJson>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct StructurizrRelationshipJson {
    pub(super) id: Option<String>,
    pub(super) source_id: Option<String>,
    pub(super) destination_id: Option<String>,
    pub(super) description: Option<String>,
    pub(super) technology: Option<String>,
    pub(super) tags: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StructurizrViewsJson {
    pub(super) system_landscape_views: Option<Vec<StructurizrViewJson>>,
    pub(super) system_context_views: Option<Vec<StructurizrViewJson>>,
    pub(super) container_views: Option<Vec<StructurizrViewJson>>,
    pub(super) component_views: Option<Vec<StructurizrViewJson>>,
    pub(super) dynamic_views: Option<Vec<StructurizrViewJson>>,
    pub(super) deployment_views: Option<Vec<StructurizrViewJson>>,
    pub(super) filtered_views: Option<Vec<StructurizrViewJson>>,
    pub(super) custom_views: Option<Vec<StructurizrViewJson>>,
    pub(super) image_views: Option<Vec<StructurizrViewJson>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct StructurizrViewJson {
    pub(super) key: Option<String>,
    pub(super) name: Option<String>,
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) elements: Vec<StructurizrViewElementJson>,
    #[serde(default, rename = "softwareSystemId")]
    pub(super) software_system_id: Option<String>,
    #[serde(default, rename = "containerId")]
    pub(super) container_id: Option<String>,
    #[serde(default, rename = "componentId")]
    pub(super) component_id: Option<String>,
    #[serde(default, rename = "elementId")]
    pub(super) element_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct StructurizrViewElementJson {
    pub(super) id: String,
}
