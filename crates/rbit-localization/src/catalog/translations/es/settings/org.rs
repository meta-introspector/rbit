//! Spanish (`es`) entries for `catalog/settings/org.rs`.

use crate::catalog::fallback::MessageWriter;
use crate::catalog::settings::org::{OrgMessage, OrgText};
use crate::plural::Plural;

entries! { Es:
    OrgMessage => MessageWriter |text| {
        OrgMessage::ProjectPriority => text.literal("Prioridad"),
        OrgMessage::ProjectNoPriority => text.literal("Sin prioridad"),
        OrgMessage::ProjectUrgent => text.literal("Urgente"),
        OrgMessage::ProjectHigh => text.literal("Alta"),
        OrgMessage::ProjectNormal => text.literal("Normal"),
        OrgMessage::ProjectLow => text.literal("Baja"),
        OrgMessage::ProjectTargetDate => text.literal("Fecha objetivo"),
        OrgMessage::ProjectOverdue => text.literal("Vencido"),
        OrgMessage::ProjectIteration => text.literal("Iteración"),
        OrgMessage::ProjectUnassigned => text.literal("Sin asignar"),
        OrgMessage::ProjectAll => text.literal("Todos"),
        OrgMessage::ProjectLayout => text.literal("Vista"),
        OrgMessage::ProjectBoardLayout => text.literal("Tablero"),
        OrgMessage::ProjectTableLayout => text.literal("Tabla"),
        OrgMessage::ProjectSort => text.literal("Ordenar por"),
        OrgMessage::ProjectAddedOrder => text.literal("ID de incidencia"),
        OrgMessage::ProjectColumn => text.literal("Columna de estado"),
        OrgMessage::ProjectEditPlanning => text.literal("Editar planificación"),
        OrgMessage::ProjectPlanningHint => text.literal("Los campos de planificación pertenecen a este proyecto. Las fechas objetivo usan UTC; el trabajo completado no está vencido."),
        OrgMessage::ProjectSavedViews => text.literal("Vistas guardadas"),
        OrgMessage::ProjectViewName => text.literal("Nombre de la vista"),
        OrgMessage::ProjectSharedView => text.literal("Compartida con miembros del proyecto"),
        OrgMessage::ProjectPersonalView => text.literal("Personal"),
        OrgMessage::ProjectSaveView => text.literal("Guardar filtros actuales"),
        OrgMessage::ProjectReplaceView => text.literal("Guardar nombre y reemplazar con filtros actuales"),
        OrgMessage::ProjectDeleteView => text.literal("Eliminar vista"),
        OrgMessage::ProjectViewsHint => text.literal("Solo tú puedes ver tus vistas personales. Las vistas compartidas respetan el acceso de cada miembro a los repositorios."),
        OrgMessage::ProjectIterations => text.literal("Iteraciones"),
        OrgMessage::ProjectIterationName => text.literal("Nombre de la iteración"),
        OrgMessage::ProjectStartsOn => text.literal("Fecha de inicio"),
        OrgMessage::ProjectEndsOn => text.literal("Fecha de fin"),
        OrgMessage::ProjectArchived => text.literal("Archivada"),
        OrgMessage::ProjectNewIteration => text.literal("Crear iteración"),
        OrgMessage::ProjectDeleteIteration => text.literal("Eliminar iteración"),
        OrgMessage::ProjectIterationsHint => text.literal("Archiva las iteraciones terminadas para conservar las asignaciones. Desasigna las tarjetas antes de eliminar una iteración; también se eliminarán sus vistas guardadas."),
        OrgMessage::ProjectProgress => text.literal("Progreso de subincidencias"),
        OrgMessage::ProjectIssue => text.literal("Incidencia o solicitud de extracción"),
        OrgMessage::ProjectNoCards => text.literal("Ninguna tarjeta coincide con estos filtros."),
        OrgMessage::ProjectBackProject => text.literal("Volver al proyecto"),
        OrgMessage::Projects => text.literal("Proyectos"),
        OrgMessage::ProjectIntro => text.literal(
            "Planifica el trabajo entre los repositorios de esta organización. Las tarjetas respetan los permisos de acceso de cada repositorio.",
        ),
        OrgMessage::ProjectEmpty => text.literal("Aún no hay proyectos."),
        OrgMessage::ProjectNew => text.literal("Nuevo proyecto"),
        OrgMessage::ProjectName => text.literal("Nombre del proyecto"),
        OrgMessage::ProjectColumns => text.literal("Columnas, una por línea"),
        OrgMessage::ProjectDefaultColumns => text.literal("Pendiente\nEn curso\nCompletado"),
        OrgMessage::ProjectColumnsHint => text.literal(
            "Usa de 2 a 12 columnas. Las incidencias completadas y las solicitudes de cambios fusionadas aparecen en la última columna.",
        ),
        OrgMessage::ProjectCreate => text.literal("Crear proyecto"),
        OrgMessage::ProjectAllRepositories => text.literal("Todos los repositorios"),
        OrgMessage::ProjectCompletionHint => text.literal(
            "Las incidencias completadas y las solicitudes de cambios fusionadas aparecen en la última columna. El trabajo reabierto vuelve a su columna guardada.",
        ),
        OrgMessage::ProjectSearchTitles => text.literal("Buscar títulos"),
        OrgMessage::ProjectRepository => text.literal("Repositorio"),
        OrgMessage::ProjectFilter => text.literal("Filtrar"),
        OrgMessage::ProjectClear => text.literal("Limpiar"),
        OrgMessage::ProjectMove => text.literal("Mover"),
        OrgMessage::ProjectAutomatic => text.literal("Completado automáticamente"),
        OrgMessage::ProjectRemoveCard => text.literal("Quitar del proyecto"),
        OrgMessage::ProjectEmptyColumn => text.literal("No hay tarjetas coincidentes en esta página."),
        OrgMessage::ProjectNextPage => text.literal("Página siguiente"),
        OrgMessage::ProjectAddThread => text.literal("Añadir una incidencia o solicitud de cambios"),
        OrgMessage::ProjectRepositoryName => text.literal("Nombre del repositorio"),
        OrgMessage::ProjectThreadNumber => text.literal("Número de incidencia o solicitud de cambios"),
        OrgMessage::ProjectAddCard => text.literal("Añadir tarjeta"),
        OrgMessage::Projectsettings => text.literal("Configuración del proyecto"),
        OrgMessage::ProjectEditHint => text.literal(
            "Las tarjetas mantienen su posición de columna cuando cambian los nombres. Mueve las tarjetas antes de eliminar sus columnas.",
        ),
        OrgMessage::ProjectSave => text.literal("Guardar proyecto"),
        OrgMessage::ProjectConfirmDelete => text.literal("Entiendo que esto elimina el proyecto y todas sus tarjetas."),
        OrgMessage::ProjectDelete => text.literal("Eliminar el proyecto y sus tarjetas"),
        OrgMessage::ProjectDeleteHint => text.literal("Las incidencias y las solicitudes de cambios se conservan."),
        OrgMessage::ChoosePermission => text.literal("Elige un permiso."),
        OrgMessage::Organization => text.literal("Organización"),
        OrgMessage::Repositories => text.literal("Repositorios"),
        OrgMessage::Members => text.literal("Miembros"),
        OrgMessage::Teams => text.literal("Equipos"),
        OrgMessage::Settings => text.literal("Configuración"),
        OrgMessage::NewOrganization => text.literal("Nueva organización"),
        OrgMessage::NewIntro => text.literal(
            "Una organización posee repositorios en nombre de un equipo. Serás su primer propietario.",
        ),
        OrgMessage::Name => text.literal("Nombre"),
        OrgMessage::NameHint => text.literal(
            "Se usa en las URL. Letras, dígitos, «-», «_» y «.». Los usuarios y las organizaciones comparten un \
             mismo espacio de nombres.",
        ),
        OrgMessage::DisplayName => text.literal("Nombre visible"),
        OrgMessage::Optional => text.literal("opcional"),
        OrgMessage::Description => text.literal("Descripción"),
        OrgMessage::Cancel => text.literal("Cancelar"),
        OrgMessage::CreateOrganization => text.literal("Crear organización"),
        OrgMessage::New => text.literal("Nuevo"),
        OrgMessage::NoRepositories => text.literal("Aún no hay repositorios"),
        OrgMessage::NoRepositoriesText => {
            text.literal("Aquí se mostrarán los repositorios de esta organización.");
        }
        OrgMessage::Profile => text.literal("Perfil"),
        OrgMessage::Website => text.literal("Sitio web"),
        OrgMessage::WebsitePlaceholder => text.literal("https://"),
        OrgMessage::SaveProfile => text.literal("Guardar perfil"),
        OrgMessage::MemberPermission => text.literal("Permiso de los miembros"),
        OrgMessage::PermissionLegend => text.literal("Lo que obtiene cada miembro en cada repositorio"),
        OrgMessage::PermissionHint => text.literal(
            "Los equipos y los colaboradores pueden ampliarlo, nunca reducirlo. Los propietarios siempre administran \
             todos los repositorios.",
        ),
        OrgMessage::SavePermission => text.literal("Guardar permiso"),
        OrgMessage::NoPermission => text.literal("Sin permiso"),
        OrgMessage::NoPermissionText => text.literal(
            "Los miembros solo ven los repositorios públicos y aquellos a los que tienen acceso sus equipos.",
        ),
        OrgMessage::Read => text.literal("Lectura"),
        OrgMessage::ReadText => text.literal("Los miembros pueden explorar y clonar todos los repositorios."),
        OrgMessage::Write => text.literal("Escritura"),
        OrgMessage::WriteText => {
            text.literal("Los miembros pueden explorar, clonar y enviar a todos los repositorios.");
        }
        OrgMessage::DangerZone => text.literal("Zona de peligro"),
        OrgMessage::DeleteOrganization => text.literal("Eliminar esta organización"),
        OrgMessage::DeleteText => text.literal(
            "Esto elimina para siempre la organización y todas sus membresías. No se puede deshacer.",
        ),
        OrgMessage::TypeToConfirmLead => text.literal("Escribe"),
        OrgMessage::TypeToConfirmTail => text.literal("para confirmar"),
        OrgMessage::DeleteOrganizationButton => text.literal("Eliminar organización"),
        OrgMessage::AddMember => text.literal("Añadir un miembro"),
        OrgMessage::Username => text.literal("Nombre de usuario"),
        OrgMessage::Role => text.literal("Rol"),
        OrgMessage::Member => text.literal("Miembro"),
        OrgMessage::Owner => text.literal("Propietario"),
        OrgMessage::Add => text.literal("Añadir"),
        OrgMessage::MembersHint => text.literal(
            "Los miembros obtienen el permiso base de la organización en todos los repositorios, más lo que les \
             concedan sus equipos. Los propietarios también pueden gestionar miembros, equipos y configuración.",
        ),
        OrgMessage::You => text.literal("(tú)"),
        OrgMessage::Leave => text.literal("Abandonar"),
        OrgMessage::Remove => text.literal("Quitar"),
        OrgMessage::MakeOwner => text.literal("Hacer propietario"),
        OrgMessage::MakeMember => text.literal("Hacer miembro"),
    }
    OrgText<'a> => MessageWriter |text| {
        OrgText::StillOwnsOne(org) => {
            text.argument(org);
            text.literal(" todavía posee 1 repositorio. Elimínalo o transfiérelo primero.");
        }
        OrgText::StillOwns(org, count) => {
            text.argument(org);
            text.literal(" todavía posee ");
            text.number(count);
            text.literal(match text.plural(count) {
                Plural::One => " repositorio. Elimínalo o transfiérelo primero.",
                _ => " repositorios. Elimínalos o transfiérelos primero.",
            });
        }
        OrgText::RemoveLabel(name) => {
            text.literal("Quitar a ");
            text.argument(name);
        }
        OrgText::RoleLabel(action, name) => {
            text.argument(action);
            text.literal(": ");
            text.argument(name);
        }
        OrgText::MembersTitle(org) => {
            text.literal("Miembros · ");
            text.argument(org);
        }
        OrgText::SettingsTitle(org) => {
            text.literal("Configuración · ");
            text.argument(org);
        }
    }
}
