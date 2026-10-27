<?php
declare(strict_types=1);

function get_login_credentials($user): array
{
    return [getenv('PANEL_DB_USER'), getenv('PANEL_DB_PASSWORD')];
}
